use super::*;
use hyper::StatusCode;
use tokio_util::{sync::CancellationToken, task::TaskTracker};

#[test]
fn zip_merge_is_deterministic_and_rejects_conflicting_assets() {
    let path = "assets/example/textures/item/token.png";
    let original = BTreeMap::from([(path.to_owned(), b"first".to_vec())]);
    let expected = encode_zip(original.clone()).expect("encode fixture");
    let mut merged = BTreeMap::new();
    let mut remaining = ResourcePackAssets::MAX_BYTES;
    add_zip(
        Cursor::new(expected.as_slice()),
        &mut merged,
        &mut remaining,
    )
    .expect("read ZIP");
    add_zip(
        Cursor::new(expected.as_slice()),
        &mut merged,
        &mut remaining,
    )
    .expect("same asset");
    assert_eq!(
        encode_zip(merged.clone()).expect("encode merged pack"),
        expected
    );
    let conflicting = encode_zip(BTreeMap::from([(path.to_owned(), b"other".to_vec())]))
        .expect("encode conflicting fixture");
    assert!(add_zip(Cursor::new(conflicting), &mut merged, &mut remaining).is_err());
    assert_eq!(merged, original);
}

#[tokio::test]
async fn embedded_and_external_modes_offer_the_exported_pack() {
    static ASSETS: std::sync::LazyLock<ResourcePackAssets> =
        std::sync::LazyLock::new(ResourcePackAssets::default);
    let directory = tempfile::tempdir().expect("test directory");
    let output = directory.path().join("pack.zip");
    let config = ResourcePackConfig {
        mode: ResourcePackMode::Optional,
        output: output.clone(),
        url: Some("http://127.0.0.1/pack".to_owned()),
        ..ResourcePackConfig::default()
    };
    let external = config
        .clone()
        .prepare(&ASSETS)
        .await
        .expect("generate ZIP")
        .expect("external offer");
    assert!(external.host.is_none());
    assert!(!external.offer.required);
    let expected = std::fs::read(&output).expect("exported ZIP");
    assert_eq!(external.offer.hash, hex::encode(Sha1::digest(&expected)));

    let hosted = ResourcePackConfig {
        mode: ResourcePackMode::Required,
        bind: Some(SocketAddr::from(([127, 0, 0, 1], 0))),
        ..config
    }
    .prepare(&ASSETS)
    .await
    .expect("generate ZIP")
    .expect("hosted offer");
    assert!(hosted.offer.required);
    assert_eq!(hosted.offer.id, external.offer.id);
    let host = hosted.host.expect("embedded listener");
    let addr = host.listener.local_addr().expect("bound address");
    let tasks = TaskTracker::new();
    let cancel = CancellationToken::new();
    let handle = tasks.spawn(host.run(cancel.clone(), tasks.clone()));
    let client = reqwest::Client::new();
    let response = client
        .get(format!("http://{addr}/pack"))
        .send()
        .await
        .expect("ZIP download");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.bytes().await.expect("downloaded bytes"), expected);
    let unrelated = client
        .get(format!("http://{addr}/other"))
        .send()
        .await
        .expect("404 response");
    assert_eq!(unrelated.status(), StatusCode::NOT_FOUND);
    cancel.cancel();
    handle.await.expect("listener stopped");
    tasks.close();
    tasks.wait().await;
}
