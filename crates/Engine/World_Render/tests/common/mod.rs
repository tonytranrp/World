//! Shared helpers for World_Render's integration tests and benchmarks:
//! CPU residency setup, a bare wgpu device on the test adapter, the WESL
//! compiler over `assets/shaders`, pipelines and blocking read-back.
//!
//! Tests may block on the GPU; the renderer never does.

#![allow(dead_code)]

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use wgpu::{BindGroupLayout, Buffer, ComputePipeline, Device, Queue};
use world_core::tree::VoxelSource;
use world_render::internals::batch::{CpuPools, UploadBatch};
use world_render::internals::key::Shape;
use world_render::internals::layout::PoolLayout;
use world_render::internals::residency::Residency;
use world_render::internals::{evict, trace, upload};
use world_testkit::policy::skip_or_fail;
use world_testkit_bevy::render::TestAdapter;

// ------------------------------------------------------------------- CPU --

/// A small pool layout for tests: two 256 KiB segments so segment handling
/// is exercised.
pub fn small_layout() -> PoolLayout {
    PoolLayout {
        node_count: 1 << 15,
        segments: vec![256 << 10, 256 << 10],
        brick_ids: 4096,
        allocs_per_segment: 8192,
        node_groups: 8192,
        upload_budget: 64 << 20,
        request_capacity: 1024,
        victim_capacity: 1024,
    }
}

/// Applies `batch` to `pools` through its serialized form.
pub fn apply_cpu(pools: &mut CpuPools, batch: &UploadBatch) {
    let mut words = Vec::new();
    batch.serialize(&mut words);
    pools.apply(&words);
}

/// Expands every node and loads every brick of `source` (unlimited budget),
/// applying each round's batch to `pools` and returning them all too.
pub fn make_resident<S: VoxelSource>(
    res: &mut Residency,
    source: &S,
    pools: &mut CpuPools,
) -> Vec<UploadBatch> {
    let mut batches = Vec::new();
    let mut batch = UploadBatch::default();
    res.reset(source, &mut batch);
    loop {
        let mut changed = false;
        for k in res.keys() {
            if k.level >= 2 && !res.is_expanded(k) {
                changed |= res.expand(source, k, &mut batch).is_ok();
            } else if k.level == 1 && !res.is_brick_resident(k) {
                changed |= res.load_brick(source, k, &mut batch).is_ok();
            }
        }
        apply_cpu(pools, &batch);
        batches.push(std::mem::take(&mut batch));
        if !changed {
            return batches;
        }
    }
}

/// A residency table and CPU pools over the small layout.
pub fn cpu_cache(shape: Shape) -> (Residency, CpuPools) {
    let layout = small_layout();
    let res = Residency::new(shape, &layout).expect("node space");
    let pools = CpuPools::new(layout.node_count, &layout.segments, layout.brick_ids);
    (res, pools)
}

// ---------------------------------------------------------------- shaders --

pub fn assets_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../assets")
}

/// Every `.wesl` file under `assets/shaders`, keyed by module components
/// (`["shaders", "traversal", "tree"]`), whatever the import origin.
struct ShaderTree(BTreeMap<Vec<String>, String>);

impl ShaderTree {
    fn load() -> Self {
        fn walk(dir: &Path, prefix: &[String], out: &mut BTreeMap<Vec<String>, String>) {
            for entry in std::fs::read_dir(dir).expect("shader folder") {
                let entry = entry.expect("dir entry");
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().into_owned();
                if path.is_dir() {
                    let mut p = prefix.to_vec();
                    p.push(name);
                    walk(&path, &p, out);
                } else if let Some(stem) = name.strip_suffix(".wesl") {
                    let mut p = prefix.to_vec();
                    p.push(stem.to_string());
                    out.insert(p, std::fs::read_to_string(&path).expect("shader source"));
                }
            }
        }
        let mut out = BTreeMap::new();
        walk(
            &assets_dir().join("shaders"),
            &["shaders".to_string()],
            &mut out,
        );
        Self(out)
    }
}

impl wesl::Resolver for ShaderTree {
    fn resolve_source<'a>(
        &'a self,
        path: &wesl::syntax::ModulePath,
    ) -> Result<Cow<'a, str>, wesl::error::ResolveError> {
        self.0
            .get(&path.components)
            .map(|s| Cow::Borrowed(s.as_str()))
            .ok_or_else(|| {
                wesl::error::ResolveError::ModuleNotFound(
                    path.clone(),
                    "not under assets/shaders".into(),
                )
            })
    }
}

/// Compiles `assets/<asset_path>` (such as `shaders/passes/primary/trace.wesl`)
/// with the options Bevy uses. Panics with the diagnostic on error.
pub fn compile_wesl(asset_path: &str) -> String {
    let tree = ShaderTree::load();
    let components = asset_path
        .trim_end_matches(".wesl")
        .split('/')
        .map(str::to_string)
        .collect();
    let main = wesl::syntax::ModulePath {
        origin: wesl::syntax::PathOrigin::Absolute,
        components,
    };
    let options = wesl::CompileOptions {
        imports: true,
        condcomp: true,
        visibility: false,
        ..Default::default()
    };
    match wesl::compile(&main, &options, &tree) {
        Ok(result) => result.to_string(),
        Err(e) => panic!("{asset_path}:\n{}", e.diagnostic().render_plain()),
    }
}

// -------------------------------------------------------------------- GPU --

/// A bare wgpu device on the adapter the environment asks for.
pub struct Gpu {
    pub device: Device,
    pub queue: Queue,
    pub info: wgpu::AdapterInfo,
    pub timestamps: bool,
}

/// The test device, or `None` after `skip_or_fail` when there is none.
pub fn gpu() -> Option<Gpu> {
    let adapter_kind = TestAdapter::from_env();
    let backends = match adapter_kind {
        TestAdapter::Gpu => wgpu::Backends::PRIMARY,
        TestAdapter::Software if cfg!(target_os = "windows") => wgpu::Backends::DX12,
        TestAdapter::Software if cfg!(target_os = "macos") => wgpu::Backends::METAL,
        TestAdapter::Software => wgpu::Backends::VULKAN,
    };
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    descriptor.backends = backends;
    // Bevy compiles D3D12 shaders with FXC unless dxcompiler.dll sits next to
    // the executable; test with the same compiler so its limits show up here.
    descriptor.backend_options.dx12.shader_compiler = wgpu::Dx12Compiler::Fxc;
    let instance = wgpu::Instance::new(descriptor);
    let options = wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        force_fallback_adapter: adapter_kind == TestAdapter::Software,
        compatible_surface: None,
        ..Default::default()
    };
    let Ok(adapter) = bevy::tasks::block_on(instance.request_adapter(&options)) else {
        skip_or_fail(&format!("no {adapter_kind:?} adapter for GPU tests"));
        return None;
    };
    let timestamps = adapter.features().contains(wgpu::Features::TIMESTAMP_QUERY);
    let required_features = if timestamps {
        wgpu::Features::TIMESTAMP_QUERY
    } else {
        wgpu::Features::empty()
    };
    let desc = wgpu::DeviceDescriptor {
        label: Some("world_render tests"),
        required_features,
        required_limits: adapter.limits(),
        ..Default::default()
    };
    match bevy::tasks::block_on(adapter.request_device(&desc)) {
        Ok((device, queue)) => Some(Gpu {
            device,
            queue,
            info: adapter.get_info(),
            timestamps,
        }),
        Err(e) => {
            skip_or_fail(&format!(
                "could not create a device on {adapter_kind:?}: {e}"
            ));
            None
        }
    }
}

/// Every pipeline the brick cache and the trace pass use.
pub struct Pipelines {
    pub trace_layout: BindGroupLayout,
    pub trace: ComputePipeline,
    pub apply_layout: BindGroupLayout,
    pub apply: Vec<ComputePipeline>,
    pub evict_layout: BindGroupLayout,
    pub evict: ComputePipeline,
}

fn compute(
    device: &Device,
    label: &str,
    source: &str,
    layout: &BindGroupLayout,
    entry: &str,
) -> ComputePipeline {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(source)),
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(label),
        bind_group_layouts: &[Some(layout)],
        immediate_size: 0,
    });
    device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some(label),
        layout: Some(&pipeline_layout),
        module: &module,
        entry_point: Some(entry),
        compilation_options: Default::default(),
        cache: None,
    })
}

impl Pipelines {
    pub fn new(device: &Device) -> Self {
        let layout = |label, entries: Vec<wgpu::BindGroupLayoutEntry>| {
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some(label),
                entries: &entries,
            })
        };
        let trace_layout = layout("trace", trace::trace_layout_entries());
        let apply_layout = layout("apply", upload::apply_layout_entries());
        let evict_layout = layout("evict", evict::pass::evict_layout_entries());
        let trace_src = compile_wesl("shaders/passes/primary/trace.wesl");
        let apply_src = compile_wesl("shaders/brick_cache/apply.wesl");
        let evict_src = compile_wesl("shaders/brick_cache/evict.wesl");
        Self {
            trace: compute(device, "trace", &trace_src, &trace_layout, "trace"),
            apply: upload::APPLY_ENTRY_POINTS
                .iter()
                .map(|e| compute(device, e, &apply_src, &apply_layout, e))
                .collect(),
            evict: compute(device, "evict", &evict_src, &evict_layout, "evict"),
            trace_layout,
            apply_layout,
            evict_layout,
        }
    }

    pub fn apply_pipelines(&self) -> upload::ApplyPipelines<'_> {
        upload::ApplyPipelines {
            unlink: &self.apply[0],
            scatter: &self.apply[1],
            table: &self.apply[2],
            link: &self.apply[3],
        }
    }
}

/// Reads a whole buffer back, blocking. Tests only.
pub fn read_words(gpu: &Gpu, buffer: &Buffer) -> Vec<u32> {
    let size = buffer.size();
    let staging = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("test readback"),
        size,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(buffer, 0, &staging, 0, size);
    gpu.queue.submit([encoder.finish()]);
    staging.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    gpu.device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .expect("poll");
    let words = {
        let view = staging.slice(..).get_mapped_range().expect("mapped");
        bytemuck::pod_collect_to_vec::<u8, u32>(&view)
    };
    staging.unmap();
    words
}

/// Waits for all submitted work.
pub fn wait(gpu: &Gpu) {
    let _ = gpu.device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: None,
    });
}

// ------------------------------------------------------------ GPU cache --

use world_render::internals::GpuNode;
use world_render::internals::GpuPools;
use world_render::internals::gbuffer::{GpuHit, HIT_WORDS, encode_rays};
use world_render::internals::trace::{MODE_RAYS, TraceParams};

/// The pools on the GPU with an upload path, as the plugin drives them.
pub struct GpuCache {
    pub pools: GpuPools,
    pub upload: upload::UploadBuffer,
    pub belt: wgpu::util::StagingBelt,
    apply_bind_group: wgpu::BindGroup,
    bind_group_generation: u64,
}

impl GpuCache {
    pub fn new(gpu: &Gpu, pipelines: &Pipelines, layout: PoolLayout) -> Self {
        let pools = GpuPools::new(&gpu.device, layout);
        let upload = upload::UploadBuffer::new(&gpu.device, 1 << 20);
        let apply_bind_group =
            upload::apply_bind_group(&gpu.device, &pipelines.apply_layout, &pools, &upload);
        Self {
            belt: wgpu::util::StagingBelt::new(gpu.device.clone(), 1 << 20),
            pools,
            upload,
            apply_bind_group,
            bind_group_generation: 0,
        }
    }

    /// Records the batch's upload and apply pass, submits and waits.
    pub fn apply(&mut self, gpu: &Gpu, pipelines: &Pipelines, batch: &UploadBatch) {
        let mut words = Vec::new();
        let counts = batch.serialize(&mut words);
        self.upload.reserve(&gpu.device, words.len() as u64 * 4);
        if self.upload.generation != self.bind_group_generation {
            self.apply_bind_group = upload::apply_bind_group(
                &gpu.device,
                &pipelines.apply_layout,
                &self.pools,
                &self.upload,
            );
            self.bind_group_generation = self.upload.generation;
        }
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        upload::record_apply(
            &mut encoder,
            &mut self.belt,
            &self.upload,
            &words,
            counts,
            &self.apply_bind_group,
            &pipelines.apply_pipelines(),
        );
        self.belt.finish_and_recall_on_submit(&encoder);
        gpu.queue.submit([encoder.finish()]);
        wait(gpu);
    }

    /// The pools read back, in the CPU model's shape.
    pub fn read(&self, gpu: &Gpu) -> CpuPools {
        let nodes: Vec<GpuNode> = bytemuck::pod_collect_to_vec(&read_words(gpu, &self.pools.nodes));
        let segments = self.pools.layout.segments.len();
        CpuPools {
            nodes,
            segments: self.pools.segments[..segments]
                .iter()
                .map(|b| read_words(gpu, b))
                .collect(),
            table: read_words(gpu, &self.pools.table),
        }
    }
}

/// What a trace needs besides the rays.
#[derive(Clone, Copy, Debug)]
pub struct TraceSetup {
    pub shape: Shape,
    pub frame: u32,
    pub pixel_angle: f32,
    pub footprint_bias: f32,
}

/// Traces `rays` (origin, direction) on the GPU in test mode; one record per ray.
pub fn gpu_trace(
    gpu: &Gpu,
    pipelines: &Pipelines,
    cache: &GpuCache,
    rays: &[([f32; 3], [f32; 3])],
    width: u32,
    setup: TraceSetup,
) -> Vec<GpuHit> {
    use wgpu::util::DeviceExt;
    let height = (rays.len() as u32).div_ceil(width);
    let mut records = encode_rays(rays.iter().copied());
    records.resize((width * height * HIT_WORDS) as usize, 0);
    let hits = gpu
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("test hits"),
            contents: bytemuck::cast_slice(&records),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        });
    let layout = &cache.pools.layout;
    let o = setup.shape.origin;
    let params = TraceParams {
        origin: [o.x, o.y, o.z, setup.shape.root_level() as i32],
        size: [width, height, setup.frame, MODE_RAYS],
        lod: [setup.pixel_angle, setup.footprint_bias, 64.0, 0.0],
        limits: [
            layout.request_capacity,
            layout.stamps_offset(),
            trace::DEFAULT_MAX_ITERATIONS,
            0,
        ],
        clip_from_world: identity(),
        world_from_clip: identity(),
        eye: [0.0; 4],
    };
    let params_buf = gpu
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("test trace params"),
            contents: bytemuck::bytes_of(&params),
            usage: wgpu::BufferUsages::UNIFORM,
        });
    let bind_group = trace::trace_bind_group(
        &gpu.device,
        &pipelines.trace_layout,
        &cache.pools,
        &hits,
        &params_buf,
    );
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    trace::record_trace(
        &mut encoder,
        &pipelines.trace,
        &bind_group,
        width,
        height,
        None,
    );
    gpu.queue.submit([encoder.finish()]);
    let words = read_words(gpu, &hits);
    words
        .chunks_exact(HIT_WORDS as usize)
        .take(rays.len())
        .filter_map(GpuHit::decode)
        .collect()
}

pub fn identity() -> [[f32; 4]; 4] {
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}
