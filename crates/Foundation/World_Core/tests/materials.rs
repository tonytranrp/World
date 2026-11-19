//! The material table against random input: valid tables survive RON exactly,
//! and any single bad field, duplicate id or duplicate name is rejected.

use proptest::prelude::*;
use world_core::material::{MaterialDef, MaterialErrors, MaterialId, MaterialTable, Rgb};

fn unit() -> impl Strategy<Value = f32> {
    0.0f32..=1.0
}

fn material(id: u16) -> impl Strategy<Value = MaterialDef> {
    (
        (unit(), unit(), unit()),
        unit(),
        0.0f32..=100.0,
        1.0f32..=25_000.0,
        0.0f32..=1_000.0,
        0.0f32..=2.0,
        unit(),
        proptest::option::of("[a-z_]{1,12}"),
    )
        .prop_map(
            move |(
                (r, g, b),
                roughness,
                emission,
                density,
                strength,
                friction,
                restitution,
                sound,
            )| {
                MaterialDef {
                    id: MaterialId(id),
                    name: format!("material_{id}"),
                    color: Rgb(r, g, b),
                    roughness,
                    emission,
                    density_kg_m3: density,
                    strength,
                    friction,
                    restitution,
                    break_sound: sound.clone(),
                    impact_sound: sound,
                }
            },
        )
}

/// 1 to 24 materials with distinct ids in 1..=2000, in random order.
fn valid_defs() -> impl Strategy<Value = Vec<MaterialDef>> {
    proptest::collection::btree_set(1u16..=2000, 1..24)
        .prop_flat_map(|ids| ids.into_iter().map(material).collect::<Vec<_>>())
        .prop_shuffle()
}

fn parse(text: &str) -> Result<MaterialTable, String> {
    ron::from_str::<MaterialTable>(text).map_err(|e| e.to_string())
}

/// Puts one field of `m` out of range. `which` picks the field.
fn break_one_field(m: &mut MaterialDef, which: usize, sign: bool) {
    let out = |lo: f32, hi: f32| if sign { hi + 0.5 + hi.abs() } else { lo - 0.5 };
    match which % 10 {
        0 => m.color.0 = out(0.0, 1.0),
        1 => m.color.2 = f32::NAN,
        2 => m.roughness = out(0.0, 1.0),
        3 => m.emission = out(0.0, 100.0),
        4 => m.density_kg_m3 = out(1.0, 25_000.0),
        5 => m.strength = out(0.0, 1_000.0),
        6 => m.friction = out(0.0, 2.0),
        7 => m.restitution = out(0.0, 1.0),
        8 => m.name = "   ".into(),
        _ => m.id = MaterialId::AIR,
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn valid_tables_round_trip_through_ron_exactly(defs in valid_defs()) {
        let table = MaterialTable::new(defs.clone()).unwrap();
        let text = ron::ser::to_string_pretty(&table, Default::default()).unwrap();
        let back = parse(&text).unwrap();
        prop_assert_eq!(&back, &table);
        for d in &defs {
            prop_assert_eq!(back.get(d.id), Some(d));
            prop_assert_eq!(back.by_name(&d.name), Some(d));
        }
        prop_assert_eq!(back.len(), defs.len());
    }

    #[test]
    fn one_bad_field_rejects_the_whole_table(
        defs in valid_defs(),
        victim in any::<prop::sample::Index>(),
        which in 0usize..10,
        sign in any::<bool>(),
    ) {
        let mut defs = defs;
        let i = victim.index(defs.len());
        break_one_field(&mut defs[i], which, sign);
        prop_assert!(MaterialTable::new(defs.clone()).is_err());
        // And the same mistake coming in through RON is rejected too.
        let text = ron::ser::to_string(&serde_shim::Spec { materials: defs }).unwrap();
        prop_assert!(parse(&text).is_err());
    }

    #[test]
    fn a_duplicate_id_or_name_is_always_reported(defs in valid_defs(), by_name in any::<bool>()) {
        let mut defs = defs;
        let mut copy = defs[0].clone();
        if by_name {
            copy.id = MaterialId(4000);
        } else {
            copy.name = "a unique name".into();
        }
        defs.push(copy);
        let MaterialErrors(errors) = MaterialTable::new(defs).unwrap_err();
        prop_assert_eq!(errors.len(), 1, "{:?}", errors);
    }

    #[test]
    fn unknown_ids_are_never_found(defs in valid_defs(), probe in any::<u16>()) {
        let table = MaterialTable::new(defs.clone()).unwrap();
        let known = defs.iter().any(|d| d.id.0 == probe);
        prop_assert_eq!(table.get(MaterialId(probe)).is_some(), known);
    }
}

/// Serializes a raw list without going through validation, to feed bad tables to the parser.
mod serde_shim {
    use serde::Serialize;
    use world_core::material::MaterialDef;

    #[derive(Serialize)]
    pub struct Spec {
        pub materials: Vec<MaterialDef>,
    }
}

#[test]
fn the_shipped_material_table_is_valid() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../assets/config/materials.ron"
    );
    let text = std::fs::read_to_string(path).expect("assets/config/materials.ron is missing");
    let table = parse(&text).unwrap_or_else(|e| panic!("materials.ron is invalid: {e}"));
    assert!(table.len() >= 3);
    assert_eq!(table.by_name("stone").map(|m| m.id), Some(MaterialId(1)));
}
