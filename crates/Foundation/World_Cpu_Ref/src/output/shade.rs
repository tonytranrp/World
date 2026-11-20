//! A flat-shaded colour image from a G-buffer, for people and golden images.

use world_core::MaterialTable;

use super::gbuffer::GBuffer;
use super::image::Image;
use crate::trace::Face;

/// The colour of a ray that hit nothing: a pale sky blue.
pub const SKY: [u8; 4] = [150, 195, 235, 255];

/// The colour of a hit whose material the table does not define: magenta,
/// so it cannot be mistaken for real content.
pub const UNKNOWN_MATERIAL: [u8; 4] = [255, 0, 255, 255];

/// Brightness per entry face, as from a sun high in the `+Y`, `+X`, `+Z`
/// octant: tops brightest, bottoms darkest, so a face's orientation is
/// readable without lighting. A ray that starts inside a solid is full
/// brightness.
pub const fn face_brightness(face: Face) -> f32 {
    match face {
        Face::PosY | Face::Inside => 1.0,
        Face::PosX => 0.85,
        Face::PosZ => 0.75,
        Face::NegZ => 0.65,
        Face::NegX => 0.6,
        Face::NegY => 0.45,
    }
}

/// Each pixel: `SKY` for a miss, `UNKNOWN_MATERIAL` for an unknown id, else
/// the material's sRGB colour times [`face_brightness`] per channel, then
/// `floor(clamp(c, 0, 1) * 255 + 0.5)`. Plain `f32` multiply and add, so the
/// bytes are identical on every machine.
pub fn shade(gbuffer: &GBuffer, materials: &MaterialTable) -> Image {
    let mut rgba = Vec::with_capacity(gbuffer.hits().len() * 4);
    for hit in gbuffer.hits() {
        let pixel = match hit {
            None => SKY,
            Some(hit) => match materials.get(hit.material) {
                None => UNKNOWN_MATERIAL,
                Some(def) => {
                    let k = face_brightness(hit.face);
                    let c = def.color;
                    [to_byte(c.0 * k), to_byte(c.1 * k), to_byte(c.2 * k), 255]
                }
            },
        };
        rgba.extend_from_slice(&pixel);
    }
    match Image::new(gbuffer.width(), gbuffer.height(), rgba) {
        Ok(image) => image,
        // A G-buffer always holds width * height hits, so the bytes fit.
        Err(e) => unreachable!("{e}"),
    }
}

/// `floor(clamp(c, 0, 1) * 255 + 0.5)` as a byte.
fn to_byte(c: f32) -> u8 {
    (c.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trace::TraceHit;
    use world_core::material::{MaterialDef, Rgb};
    use world_core::{MaterialId, VoxelPos};

    fn table() -> MaterialTable {
        MaterialTable::new(vec![MaterialDef {
            id: MaterialId(1),
            name: "stone".into(),
            color: Rgb(1.0, 0.5, 0.0),
            roughness: 1.0,
            emission: 0.0,
            density_kg_m3: 2000.0,
            strength: 1.0,
            friction: 1.0,
            restitution: 0.0,
            break_sound: None,
            impact_sound: None,
        }])
        .unwrap()
    }

    fn hit(material: u16, face: Face) -> Option<TraceHit> {
        Some(TraceHit {
            voxel: VoxelPos::ORIGIN,
            level: 0,
            face,
            material: MaterialId(material),
            resident: true,
            t: 1.0,
        })
    }

    #[test]
    fn misses_unknowns_and_faces_are_coloured_as_documented() {
        let g = GBuffer::new(
            4,
            1,
            vec![
                None,
                hit(1, Face::PosY),
                hit(1, Face::NegY),
                hit(7, Face::PosY),
            ],
        )
        .unwrap();
        let img = shade(&g, &table());
        assert_eq!(img.pixel(0, 0), Some(SKY));
        assert_eq!(img.pixel(1, 0), Some([255, 128, 0, 255]));
        assert_eq!(img.pixel(2, 0), Some([115, 57, 0, 255]));
        assert_eq!(img.pixel(3, 0), Some(UNKNOWN_MATERIAL));
    }
}
