use terra_world::{
    level::{LEVEL_SCHEMA_VERSION, LevelArtifactError, LevelData},
    terrain::Terrain,
};

#[test]
fn world_classifications_and_artifact_codec_are_available_from_the_model_crate() {
    assert_eq!(Terrain::ALL.len(), 20);
    assert!(Terrain::Lake.is_water());
    assert!(Terrain::Plains.is_land_biome());
    assert_eq!(LEVEL_SCHEMA_VERSION, 1);
    assert!(matches!(
        LevelData::from_artifact_bytes(b"legacy"),
        Err(LevelArtifactError::MissingHeader)
    ));
}
