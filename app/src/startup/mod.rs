//! Boot order, one responsibility per step:
//!
//! 1. parse the command line and resolve configuration
//! 2. choose the backend set the OS offers
//! 3. read adapter features and limits (no device yet)
//! 4. choose the adapter and capability tier, apply the profile
//! 5. configure the device request and task pools
//! 6. register plugins and run
//!
//! The device and tier are decided once; changing them needs a restart.

#[cfg(feature = "hotpatch")]
mod hot_probe;
mod select;

use std::process::ExitCode;

use bevy::app::{TaskPoolOptions, TaskPoolPlugin};
use bevy::prelude::*;
use bevy::render::RenderPlugin;
use bevy::render::settings::{Backends, PowerPreference, WgpuSettings};
use clap::Parser;
use world_anim::prelude::WorldAnimPlugin;
use world_game::prelude::WorldGamePlugin;
use world_physics::prelude::WorldPhysicsPlugin;
use world_reload::prelude::WorldReloadPlugin;
use world_render::prelude::{
    AdapterReport, Budgets, RenderTier, Tier, WorldRenderPlugin, classify, probe_adapters,
};
use world_sim::prelude::WorldSimPlugin;

use crate::cli::Cli;
use crate::settings::Config;
use select::{Chosen, Rejection};

pub fn run() -> ExitCode {
    let config = Config::from_cli(&Cli::parse());
    match boot(&config) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::from(2)
        }
    }
}

fn boot(config: &Config) -> Result<(), String> {
    let backends = config.backend.to_backends()?;
    let reports = probe_adapters(backends);

    if config.list_adapters {
        print_adapters(&reports, config);
        return Ok(());
    }

    let chosen = select::choose(&reports, config.profile)
        .map_err(|rejections| unsupported_message(backends, &rejections))?;
    let tier = render_tier(&chosen, config);

    let mut app = assemble(config, &chosen, tier);
    match app.run() {
        AppExit::Success => Ok(()),
        AppExit::Error(code) => Err(format!("the game exited with code {code}")),
    }
}

fn render_tier(chosen: &Chosen, config: &Config) -> RenderTier {
    RenderTier {
        tier: chosen.classification.tier,
        capabilities: chosen.classification.capabilities,
        profile: config.profile,
        budgets: Budgets::for_profile(config.profile, &chosen.report),
        backend: chosen.report.backend,
        adapter_name: chosen.report.name.clone(),
    }
}

fn assemble(config: &Config, chosen: &Chosen, tier: RenderTier) -> App {
    // Pin Bevy to exactly the adapter we classified, so the tier we logged is the device we get.
    let wgpu_settings = WgpuSettings {
        backends: Some(Backends::from(chosen.report.backend)),
        adapter_name: Some(chosen.report.name.clone()),
        power_preference: PowerPreference::HighPerformance,
        ..default()
    };
    let task_pools = match tier.budgets.worker_threads {
        Some(threads) => TaskPoolOptions::with_num_threads(threads),
        None => TaskPoolOptions::default(),
    };

    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(RenderPlugin {
                render_creation: wgpu_settings.into(),
                ..default()
            })
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: "World".into(),
                    ..default()
                }),
                ..default()
            })
            .set(TaskPoolPlugin {
                task_pool_options: task_pools,
            }),
    )
    .add_plugins((
        WorldReloadPlugin,
        WorldRenderPlugin::new(tier),
        WorldPhysicsPlugin,
        WorldAnimPlugin,
        WorldSimPlugin,
        WorldGamePlugin,
    ));

    #[cfg(feature = "hotpatch")]
    hot_probe::build(&mut app);

    if let Some(frames) = config.exit_after_frames {
        app.insert_resource(ExitAfter(frames))
            .add_systems(Update, exit_after_frames);
    }
    app
}

#[derive(Resource)]
struct ExitAfter(u32);

fn exit_after_frames(
    limit: Res<ExitAfter>,
    mut frames: Local<u32>,
    mut exit: MessageWriter<AppExit>,
) {
    *frames += 1;
    if *frames >= limit.0 {
        exit.write(AppExit::Success);
    }
}

fn print_adapters(reports: &[AdapterReport], config: &Config) {
    println!(
        "backend request: {:?}, profile: {:?}",
        config.backend, config.profile
    );
    if reports.is_empty() {
        println!("no adapters found");
    }
    for (i, r) in reports.iter().enumerate() {
        let verdict = match classify(r) {
            Ok(c) => {
                let c = c.with_profile(config.profile);
                let tier = match c.tier {
                    Tier::A => "A",
                    Tier::B => "B",
                    Tier::C => "C",
                };
                format!(
                    "tier {tier} (ray_query={}, mesh_shader={})",
                    c.capabilities.ray_query, c.capabilities.mesh_shader
                )
            }
            Err(reason) => format!("unsupported: {reason}"),
        };
        println!(
            "[{i}] {} | {:?} | {:?} | {verdict}",
            r.name, r.backend, r.device_type
        );
    }
}

fn unsupported_message(backends: Backends, rejections: &[Rejection]) -> String {
    if rejections.is_empty() {
        return format!(
            "no GPU adapter was found for {backends:?}. World needs a DX12, Vulkan or Metal GPU \
             with compute shader support. Check your graphics drivers."
        );
    }
    let mut message = String::from(
        "no compatible GPU found. World needs compute shaders and storage buffers (tier B or better).",
    );
    for r in rejections {
        message.push_str(&format!("\n  - {} ({:?}): {}", r.name, r.backend, r.reason));
    }
    message
}
