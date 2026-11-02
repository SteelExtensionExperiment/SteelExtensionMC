use super::*;
use simdnbt::owned::read_compound;
use std::io::Cursor;

#[test]
fn dependent_writes_preserve_unrelated_namespaces_and_failed_spends() -> Result<(), String> {
    let mut data = CustomData::default();
    let mana = DataPath::parse("magic:state/mana/current")?;
    let roster = DataPath::parse("creatures:state/selected")?;
    data.set(&mana, NbtTag::Double(25.0))?;
    data.set(&roster, NbtTag::Int(2))?;
    assert!(data.spend(&mana, 20.0)?);
    assert!(!data.spend(&mana, 20.0)?);
    assert_eq!(data.get(&mana).and_then(CustomData::number), Some(5.0));
    assert_eq!(data.get(&roster), Some(&NbtTag::Int(2)));
    assert!(
        data.set(
            &DataPath::parse("magic:state/mana/current/broken")?,
            NbtTag::Int(1)
        )
        .is_err()
    );
    let mut bytes = Vec::new();
    data.snapshot().write(&mut bytes);
    let restored =
        read_compound(&mut Cursor::new(bytes.as_slice())).map_err(|e| format!("{e:?}"))?;
    let restored = CustomData::from_compound(restored);
    assert_eq!(restored.get(&mana), data.get(&mana));
    assert_eq!(restored.get(&roster), data.get(&roster));
    Ok(())
}

#[test]
fn root_replacement_and_invalid_paths_cannot_corrupt_other_data() -> Result<(), String> {
    let mut data = CustomData::default();
    assert!(DataPath::parse("mana").is_err());
    assert!(DataPath::parse("minecraft:mana").is_err());
    assert!(DataPath::parse("magic:state//mana").is_err());
    let path = DataPath::parse("magic:state")?;
    assert!(data.set(&path, NbtTag::Double(10.0)).is_err());
    assert!(data.get(&path).is_none());
    Ok(())
}

#[test]
fn namespaced_mutations_preserve_native_fields_and_snapshot_independence() -> Result<(), String> {
    let mut native = NbtCompound::new();
    native.insert("flag", NbtTag::Byte(1));
    native.insert("native:scalar", NbtTag::String("untouched".into()));
    let mut data = CustomData::from_compound(native);
    let path = DataPath::parse("example:state/mana/current")?;
    data.set(&path, NbtTag::Int(10))?;
    let snapshot = data.snapshot();
    assert!(data.spend(&path, 3.0)?);
    assert_eq!(data.get(&path).and_then(CustomData::number), Some(7.0));
    assert_eq!(
        snapshot
            .compound("example:state")
            .and_then(|root| root.compound("mana"))
            .and_then(|gauge| gauge.int("current")),
        Some(10)
    );
    assert_eq!(data.as_compound().byte("flag"), Some(1));
    assert_eq!(
        data.as_compound()
            .string("native:scalar")
            .map(ToString::to_string),
        Some("untouched".into())
    );
    assert!(
        data.set(&DataPath::parse("native:scalar/child")?, NbtTag::Int(3))
            .is_err()
    );
    assert_eq!(
        data.as_compound()
            .string("native:scalar")
            .map(ToString::to_string),
        Some("untouched".into())
    );
    Ok(())
}
