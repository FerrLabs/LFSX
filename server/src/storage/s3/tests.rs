use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use futures_util::StreamExt;
use sha2::Digest;

use super::*;

pub(crate) type Objects = Arc<Mutex<HashMap<String, Vec<u8>>>>;

// Enough of S3 to prove the layout and the wire format this store depends on:
// PUT, HEAD, ranged GET and a prefix listing. The same shape as the stub forge
// the authentication tests run against — a real MinIO belongs in CI, not in the
// path of every `cargo test`.
pub(crate) async fn bucket() -> (String, Objects) {
    stub(true, true, None).await
}

// A store where another repository turns up in the middle of a collection: the
// given key appears the first time this sees a delete, which is the moment a
// sweep has committed to dropping a marker and has not yet touched the bytes.
//
// It is the one interleaving that matters and the one a test cannot otherwise
// produce, since both sides are a handful of requests wide.
pub(crate) async fn bucket_where_a_claim_arrives_mid_sweep(key: &str) -> (String, Objects) {
    stub(true, true, Some(key.to_owned())).await
}

// The store the checksum probe exists to find: it takes the header, stores the
// bytes, and never compares the two. Several S3-compatible implementations
// accept `x-amz-checksum-*` this way, which is why the server asks rather than
// assumes.
pub(crate) async fn bucket_ignoring_checksums() -> (String, Objects) {
    stub(false, true, None).await
}

// And the one the locking probe exists to find: it accepts `If-None-Match: *`
// and writes anyway, so two callers racing for the same lock are both told they
// took it.
pub(crate) async fn bucket_ignoring_conditions() -> (String, Objects) {
    stub(true, false, None).await
}

async fn stub(
    enforces_checksums: bool,
    enforces_conditions: bool,
    arriving: Option<String>,
) -> (String, Objects) {
    let objects: Objects = Arc::new(Mutex::new(HashMap::new()));
    let arriving = Arc::new(Mutex::new(arriving));

    let app = Router::new()
        .route(
            "/{*key}",
            any(
                move |State(objects): State<Objects>,
                      Path(key): Path<String>,
                      axum::extract::RawQuery(query): axum::extract::RawQuery,
                      headers: HeaderMap,
                      method: axum::http::Method,
                      body: axum::body::Body| {
                    let arriving = arriving.clone();
                    async move {
                        let query = query.unwrap_or_default();

                        // Path-style addressing puts the bucket in the path, so the
                        // stub drops it to keep the same keyspace a real bucket has.
                        let key = key.strip_prefix("assets/").unwrap_or(&key).to_owned();

                        if query.contains("list-type=2") {
                            return list(&objects, &query);
                        }

                        match method {
                            axum::http::Method::PUT => {
                                let bytes = axum::body::to_bytes(body, usize::MAX)
                                    .await
                                    .unwrap_or_default();

                                // A conforming store refuses a body that does not
                                // match the checksum the URL was signed for, and the
                                // whole pre-signed upload path rests on it, so the
                                // stub does it too rather than accepting everything
                                // and letting the tests pass for the wrong reason.
                                if enforces_checksums && !matches(&headers, &bytes) {
                                    return (
                                        StatusCode::BAD_REQUEST,
                                        "XAmzContentChecksumMismatch",
                                    )
                                        .into_response();
                                }

                                // `If-None-Match: *` is the whole of lock uniqueness
                                // against a bucket: the store is the only thing that
                                // can say which of two callers arrived second. A
                                // store without it answers success twice, which is
                                // what `enforces_conditions` stands in for.
                                if enforces_conditions
                                    && headers.contains_key("if-none-match")
                                    && objects.lock().unwrap().contains_key(&key)
                                {
                                    return StatusCode::PRECONDITION_FAILED.into_response();
                                }

                                objects.lock().unwrap().insert(key, bytes.to_vec());
                                StatusCode::OK.into_response()
                            }
                            axum::http::Method::HEAD => match objects.lock().unwrap().get(&key) {
                                Some(object) => (
                                    StatusCode::OK,
                                    [(header::CONTENT_LENGTH, object.len().to_string())],
                                )
                                    .into_response(),
                                None => StatusCode::NOT_FOUND.into_response(),
                            },
                            // S3 answers 204 whether or not the key was there, and
                            // the store depends on that: it settles whether a delete
                            // removed anything with a HEAD beforehand rather than
                            // reading the status.
                            axum::http::Method::DELETE => {
                                objects.lock().unwrap().remove(&key);

                                // Once, on the first delete of the run.
                                if let Some(key) = arriving.lock().unwrap().take() {
                                    objects.lock().unwrap().insert(key, Vec::new());
                                }

                                StatusCode::NO_CONTENT.into_response()
                            }
                            _ => get(&objects, &key, &headers),
                        }
                    }
                },
            ),
        )
        .with_state(objects.clone());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    (format!("http://{address}"), objects)
}

// A request with no checksum header is nothing to disagree with. One that has it
// has to hash to it.
fn matches(headers: &HeaderMap, bytes: &[u8]) -> bool {
    let Some(claimed) = headers.get(CHECKSUM) else {
        return true;
    };

    let actual = base64::engine::general_purpose::STANDARD.encode(sha2::Sha256::digest(bytes));

    claimed.as_bytes() == actual.as_bytes()
}

fn get(objects: &Objects, key: &str, headers: &HeaderMap) -> Response {
    let held = objects.lock().unwrap();
    let Some(object) = held.get(key) else {
        return StatusCode::NOT_FOUND.into_response();
    };

    let range = headers
        .get(header::RANGE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("bytes="))
        .and_then(|value| value.split_once('-'));

    let Some((start, end)) = range else {
        return (StatusCode::OK, object.clone()).into_response();
    };

    let start: usize = start.parse().unwrap_or_default();
    let end: usize = end.parse().unwrap_or(object.len() - 1);
    let slice = object[start.min(object.len())..(end + 1).min(object.len())].to_vec();

    (StatusCode::PARTIAL_CONTENT, slice).into_response()
}

fn list(objects: &Objects, query: &str) -> Response {
    let prefix = query
        .split('&')
        .find_map(|pair| pair.strip_prefix("prefix="))
        .map(|prefix| prefix.replace("%2F", "/"))
        .unwrap_or_default();

    let held = objects.lock().unwrap();
    let matching: Vec<_> = held.keys().filter(|key| key.starts_with(&prefix)).collect();
    let keys: String = matching
        .iter()
        .map(|key| format!(
            "<Contents><Key>{key}</Key><Size>0</Size><ETag>\"d41d8\"</ETag><LastModified>2026-08-15T00:00:00.000Z</LastModified><StorageClass>STANDARD</StorageClass></Contents>"
        ))
        .collect();

    (
        StatusCode::OK,
        format!(
            "<?xml version=\"1.0\"?><ListBucketResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"><Name>assets</Name><KeyCount>{}</KeyCount><MaxKeys>1000</MaxKeys><IsTruncated>false</IsTruncated>{keys}</ListBucketResult>",
            matching.len()
        ),
    )
        .into_response()
}

pub(crate) fn store(endpoint: &str) -> S3Store {
    configured(endpoint, false)
}

pub(crate) fn redirecting(endpoint: &str) -> S3Store {
    configured(endpoint, true)
}

fn configured(endpoint: &str, redirect: bool) -> S3Store {
    S3Store::new(keyspace(endpoint), redirect)
}

pub(crate) fn keyspace(endpoint: &str) -> Keyspace {
    Keyspace::new(&S3Config {
        endpoint: endpoint.to_owned(),
        bucket: "assets".into(),
        region: "us-east-1".into(),
        access_key: "key".into(),
        secret_key: "secret".into(),
        path_style: true,
        lifetime: Duration::from_secs(1800),
    })
    .unwrap()
}

fn namespace(repo: &str) -> Namespace {
    Namespace::new("FerrLabs", repo).unwrap()
}

async fn staged(payload: &[u8]) -> (tempfile::TempDir, std::path::PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("staged");
    tokio::fs::write(&path, payload).await.unwrap();

    (root, path)
}

async fn read_all(store: &S3Store, oid: &str, start: u64, length: u64) -> Vec<u8> {
    let mut chunks = Box::pin(store.read(oid, start, length).await.unwrap());
    let mut out = Vec::new();

    while let Some(chunk) = chunks.next().await {
        out.extend_from_slice(&chunk.unwrap());
    }

    out
}

const OID: &str = "5891b5b522d5df086d0ff0b110fbd9d21bb4fc7163af34d08286a2e846f6be03";

#[tokio::test]
async fn an_object_goes_up_once_and_comes_back_whole() {
    let (endpoint, objects) = bucket().await;
    let store = store(&endpoint);
    let payload = b"an asset that lives in a bucket".repeat(64);
    let (_root, path) = staged(&payload).await;

    store
        .store(&namespace("Blastlands"), OID, &path)
        .await
        .unwrap();

    assert_eq!(
        read_all(&store, OID, 0, payload.len() as u64).await,
        payload
    );
    assert_eq!(
        objects.lock().unwrap().len(),
        3,
        "the bytes under their digest, one empty marker saying this repository holds them, and one \
         index entry so a sweep can find that marker from the digest alone"
    );
}

#[tokio::test]
async fn a_second_repository_adds_a_marker_and_not_the_bytes() {
    let (endpoint, objects) = bucket().await;
    let store = store(&endpoint);
    let payload = b"the asset pack both projects use".repeat(32);
    let (_root, path) = staged(&payload).await;

    store
        .store(&namespace("Blastlands"), OID, &path)
        .await
        .unwrap();
    store.store(&namespace("Arena"), OID, &path).await.unwrap();

    let held = objects.lock().unwrap();
    assert_eq!(
        held.len(),
        5,
        "one copy of the bytes, two markers and the two index entries that point back at them. \
         Deduplication is what content addressing gives for free here, and paying twice would \
         throw it away"
    );
    assert_eq!(held.values().filter(|object| !object.is_empty()).count(), 1);
}

#[tokio::test]
async fn a_repository_that_never_pushed_an_object_does_not_hold_it() {
    let (endpoint, _objects) = bucket().await;
    let store = store(&endpoint);
    let (_root, path) = staged(b"an asset one project pushed").await;
    store
        .store(&namespace("Blastlands"), OID, &path)
        .await
        .unwrap();

    assert!(store.exists(&namespace("Blastlands"), OID).await);
    assert!(
        !store.exists(&namespace("Arena"), OID).await,
        "the marker is the proof of possession — without it, guessing a digest would be enough to \
         read another project's assets out of the shared keyspace"
    );
}

#[tokio::test]
async fn a_range_asks_the_bucket_for_that_range() {
    let (endpoint, _objects) = bucket().await;
    let store = store(&endpoint);
    let payload: Vec<u8> = (0..=255u8).cycle().take(8192).collect();
    let (_root, path) = staged(&payload).await;
    store
        .store(&namespace("Blastlands"), OID, &path)
        .await
        .unwrap();

    assert_eq!(read_all(&store, OID, 4096, 100).await, payload[4096..4196]);
}

#[tokio::test]
async fn what_a_repository_holds_is_counted_from_its_markers() {
    let (endpoint, _objects) = bucket().await;
    let store = store(&endpoint);
    let (_root, path) = staged(b"an asset worth counting").await;
    store
        .store(&namespace("Blastlands"), OID, &path)
        .await
        .unwrap();

    let (objects, bytes) = store.usage_of(&namespace("Blastlands")).await;

    assert_eq!(objects, 1);
    assert_eq!(
        bytes, 23,
        "the marker is empty, so the size has to come from the content it points at — counting          the marker would report a repository holding nothing"
    );
}

#[tokio::test]
async fn an_object_id_that_could_not_be_one_is_refused_rather_than_slicing_into_it() {
    let (endpoint, _objects) = bucket().await;
    let store = store(&endpoint);
    let (_root, path) = staged(b"bytes nobody will store").await;

    // The fanout takes the first four characters of the digest, so anything
    // shorter is an index out of bounds — a panic where the client deserves a
    // refusal.
    for short in ["", "ab", "abc"] {
        assert!(store.size_of(short).await.is_err());
        assert!(store.read(short, 0, 1).await.is_err());
        assert!(
            store
                .store(&namespace("Blastlands"), short, &path)
                .await
                .is_err()
        );
    }
}

#[test]
fn a_store_that_was_not_asked_to_redirect_hands_out_no_signature() {
    assert!(
        store("http://127.0.0.1:1")
            .presigned_download(OID)
            .is_none(),
        "streaming through the server is what counts the bytes, serves the ranges and holds the          ceiling — giving that up is a choice an operator makes, not a default they discover"
    );
}

#[test]
fn a_pre_signed_url_points_at_the_shared_content_key_and_expires() {
    let signed = redirecting("http://s3.example")
        .presigned_download(OID)
        .expect("a redirecting store signs");

    assert!(
        signed.contains(&format!(".content/{}/{}/{OID}", &OID[0..2], &OID[2..4])),
        "the bytes live once, under their digest: {signed}"
    );
    assert!(
        !signed.contains("FerrLabs"),
        "the marker is this server's bookkeeping and has no business in a URL the client          resolves: {signed}"
    );
    assert!(signed.contains("X-Amz-Signature="), "{signed}");
    assert!(
        signed.contains("X-Amz-Expires=1800"),
        "a client told the action lasts half an hour and handed a URL that dies sooner fails a          resume it had every reason to expect: {signed}"
    );
}

#[test]
fn an_object_id_that_could_not_be_one_is_never_signed_into_a_key() {
    let store = redirecting("http://s3.example");

    // The fanout slices the first four characters, so a short oid is a panic
    // rather than a refusal — and a signature is handed to the client, which is
    // the last place to discover it.
    for short in ["", "ab", "abc", "../../etc/passwd"] {
        assert!(store.presigned_download(short).is_none(), "{short:?}");
    }
}

// A second digest, so a test can tell an object that was collected apart from
// one that merely was not looked at.
const OTHER_OID: &str = "6b86b273ff34fce19d6b804eff5a3f5747ada4eaa22f1d49c01e52ddb7875b4b";

async fn collect(store: &S3Store, repo: &str) -> crate::storage::SweepReport {
    store
        .sweep(
            &namespace(repo),
            &std::collections::HashSet::new(),
            Duration::from_secs(0),
            false,
        )
        .await
        .unwrap()
}

fn holds(objects: &Objects, key: &str) -> bool {
    objects.lock().unwrap().contains_key(key)
}

// Two repositories, one object, and the bytes may only go with the second claim.
// This is the whole point of the marker keyspace, and the index has to answer it
// the same way the whole-bucket listing did.
//
// It also crosses both paths in one run. The first sweep finds no index and reads
// the bucket, which builds one; the second finds it and asks a single prefix.
#[tokio::test]
async fn the_bytes_go_when_the_last_repository_holding_them_lets_go() {
    let (endpoint, objects) = bucket().await;
    let store = store(&endpoint);
    let payload = b"an asset pack two projects share".repeat(16);
    let (_root, path) = staged(&payload).await;

    store
        .store(&namespace("Blastlands"), OID, &path)
        .await
        .unwrap();
    store.store(&namespace("Arena"), OID, &path).await.unwrap();

    collect(&store, "Blastlands").await;

    assert!(
        holds(&objects, &S3Store::content_key(OID)),
        "Arena still holds the object, so dropping Blastlands' claim frees nothing"
    );

    let report = collect(&store, "Arena").await;

    assert!(!holds(&objects, &S3Store::content_key(OID)));
    assert_eq!(
        report.bytes,
        payload.len() as u64,
        "the last claim is the one that frees the bytes, and it is the one that reports them"
    );
    assert!(
        !objects
            .lock()
            .unwrap()
            .keys()
            .any(|key| key.starts_with(&format!(".refs/{OID}/"))),
        "an object that is gone must leave no claim behind: a ref outliving its bytes would keep \\
         the next copy of them from ever being collected"
    );
}

// The migration hazard, and the reason the index is not trusted on sight.
//
// A bucket written before the index existed has markers and no refs. Asking the
// index about an object two repositories hold would find nothing, read that as
// nobody claiming it, and delete the bytes out from under the other one. Only
// something that has walked the whole bucket may say the index is complete.
#[tokio::test]
async fn a_bucket_that_predates_the_index_is_never_swept_against_it() {
    let (endpoint, objects) = bucket().await;
    let store = store(&endpoint);

    {
        let mut held = objects.lock().unwrap();
        held.insert(S3Store::content_key(OID), b"the shared asset".to_vec());
        held.insert(
            S3Store::marker_key(&namespace("Blastlands"), OID),
            Vec::new(),
        );
        held.insert(S3Store::marker_key(&namespace("Arena"), OID), Vec::new());
    }

    collect(&store, "Blastlands").await;

    assert!(
        holds(&objects, &S3Store::content_key(OID)),
        "Arena's claim predates the index and has no ref, so only reading the whole bucket can see \\
         it, and missing it deletes an object Arena still holds"
    );
    assert!(!holds(
        &objects,
        &S3Store::marker_key(&namespace("Blastlands"), OID)
    ));
}

// And the pass that had to read the whole bucket leaves an index covering every
// repository in it, not just the one that swept. A ref missing for Arena is
// exactly how the next sweep would free bytes Arena holds.
#[tokio::test]
async fn the_pass_that_reads_the_whole_bucket_leaves_an_index_behind() {
    let (endpoint, objects) = bucket().await;
    let store = store(&endpoint);

    {
        let mut held = objects.lock().unwrap();
        held.insert(S3Store::content_key(OID), b"the shared asset".to_vec());
        held.insert(
            S3Store::marker_key(&namespace("Blastlands"), OID),
            Vec::new(),
        );
        held.insert(S3Store::marker_key(&namespace("Arena"), OID), Vec::new());
        held.insert(
            S3Store::marker_key(&namespace("Arena"), OTHER_OID),
            Vec::new(),
        );
    }

    // Retained, so nothing is deleted and what is left is the index alone.
    store
        .sweep(
            &namespace("Blastlands"),
            &std::collections::HashSet::from([OID.to_owned()]),
            Duration::from_secs(0),
            false,
        )
        .await
        .unwrap();

    let held = objects.lock().unwrap();

    assert!(held.contains_key(".refs/.complete"));
    assert!(held.contains_key(&refs::key(&namespace("Blastlands"), OID)));
    assert!(held.contains_key(&refs::key(&namespace("Arena"), OID)));
    assert!(
        held.contains_key(&refs::key(&namespace("Arena"), OTHER_OID)),
        "every marker in the bucket earns a ref, including the ones belonging to repositories this \\
         sweep never touched"
    );
}

// A dry run reports what a real one would free and writes nothing, index
// included. Building the index is a write, and an operator asking what a sweep
// would do has not agreed to one.
#[tokio::test]
async fn a_dry_run_leaves_the_bucket_exactly_as_it_found_it() {
    let (endpoint, objects) = bucket().await;
    let store = store(&endpoint);

    {
        let mut held = objects.lock().unwrap();
        held.insert(S3Store::content_key(OID), b"the shared asset".to_vec());
        held.insert(
            S3Store::marker_key(&namespace("Blastlands"), OID),
            Vec::new(),
        );
    }

    let before = objects.lock().unwrap().clone();

    let report = store
        .sweep(
            &namespace("Blastlands"),
            &std::collections::HashSet::new(),
            Duration::from_secs(0),
            true,
        )
        .await
        .unwrap();

    assert_eq!(report.swept, 1);
    assert_eq!(*objects.lock().unwrap(), before);
}

// The asymmetry the index is built around, stated as a test. A store that cannot
// be asked is not an answer of "nobody holds this": a false yes leaves an object
// nobody reads, a false no destroys one somebody does.
#[tokio::test]
async fn an_index_that_cannot_be_read_answers_that_somebody_still_holds_the_object() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let closed = listener.local_addr().unwrap();
    drop(listener);

    let keys = keyspace(&format!("http://{closed}"));

    assert!(
        refs::claimed_by_another(&keys, &namespace("Blastlands"), OID).await,
        "an unreachable store must never be read as permission to delete"
    );
}

// A pre-signed upload is one PUT to one href, because that is all the `basic`
// transfer adapter can do, so the 5 GiB ceiling on a single request is real and
// permanent there. Withholding the URL is not a refusal: the object falls back
// to coming through this server, which sends it in parts.
#[tokio::test]
async fn an_object_too_big_for_one_request_is_not_given_an_upload_url() {
    let store = redirecting("http://127.0.0.1:1");
    let ns = namespace("Blastlands");
    let ceiling = super::multipart::SINGLE_PUT_CEILING;

    assert!(store.presigned_upload(&ns, OID, ceiling).is_some());
    assert!(
        store.presigned_upload(&ns, OID, ceiling + 1).is_none(),
        "a client handed this URL would upload for an hour and be refused by the bucket at the end"
    );
}

// The race #177 records. A sweep decides an object is unclaimed, and before it
// gets to the bytes another repository pushes the same digest: it finds the
// content already there, skips the upload, and writes a claim. Deleting now
// leaves that repository holding a marker pointing at nothing, which its client
// meets as a missing object on the next pull.
//
// So the index gets the last word, asked again immediately before the delete.
#[tokio::test]
async fn a_claim_that_arrives_mid_sweep_keeps_the_bytes() {
    let arena = refs::key(&namespace("Arena"), OID);
    let (endpoint, objects) = bucket_where_a_claim_arrives_mid_sweep(&arena).await;
    let store = store(&endpoint);
    let payload = b"an asset pack a second project is about to want".repeat(8);
    let (_root, path) = staged(&payload).await;

    store
        .store(&namespace("Blastlands"), OID, &path)
        .await
        .unwrap();

    let report = collect(&store, "Blastlands").await;

    assert!(
        holds(&objects, &S3Store::content_key(OID)),
        "Arena claimed the object while this sweep was dropping Blastlands' marker, and its bytes \
         are the ones Arena skipped uploading"
    );
    assert_eq!(
        report.bytes, 0,
        "nothing was freed, and a report saying otherwise promises space the bucket still holds"
    );
    assert!(!holds(
        &objects,
        &S3Store::marker_key(&namespace("Blastlands"), OID)
    ));
}

// And the same on the indexed path, which is the one a bucket takes once
// anything has walked it. The two paths decide `frees` from different evidence,
// a per-object listing here and a whole-bucket one there, so both have to ask
// again rather than trusting what they worked out earlier.
#[tokio::test]
async fn a_claim_that_arrives_mid_sweep_keeps_the_bytes_on_the_indexed_path_too() {
    let arena = refs::key(&namespace("Arena"), OID);
    let (endpoint, objects) = bucket_where_a_claim_arrives_mid_sweep(&arena).await;
    let store = store(&endpoint);
    let (_root, path) = staged(b"an asset pack two projects share").await;

    store
        .store(&namespace("Blastlands"), OID, &path)
        .await
        .unwrap();

    // What a bucket looks like once a sweep has built the index, which is what
    // sends the next one down the per-object path.
    objects
        .lock()
        .unwrap()
        .insert(".refs/.complete".to_owned(), Vec::new());

    let report = collect(&store, "Blastlands").await;

    assert!(holds(&objects, &S3Store::content_key(OID)));
    assert_eq!(report.bytes, 0);
}

// And what saves the bytes is a claim, not merely something happening mid-sweep.
// Without this the two above would pass against a check that refused to delete
// whenever the bucket changed underneath it, which would make collection stop
// working on any store that is busy.
#[tokio::test]
async fn something_that_is_not_a_claim_arriving_mid_sweep_frees_the_bytes_anyway() {
    let (endpoint, objects) =
        bucket_where_a_claim_arrives_mid_sweep(".incoming/FerrLabs/Arena/an-upload").await;
    let store = store(&endpoint);
    let (_root, path) = staged(b"an asset only one project ever held").await;

    store
        .store(&namespace("Blastlands"), OID, &path)
        .await
        .unwrap();

    let report = collect(&store, "Blastlands").await;

    assert_eq!(report.swept, 1);
    assert!(
        !holds(&objects, &S3Store::content_key(OID)),
        "nothing claimed this object, so a write elsewhere in the bucket is not a reason to keep it"
    );
}

// The sizes are gathered a few at a time and added up out of order, so one
// object proves nothing: a fold that dropped or doubled one would still pass on
// a repository holding a single asset.
#[tokio::test]
async fn what_a_repository_holds_is_the_sum_of_every_object_in_it() {
    let (endpoint, _objects) = bucket().await;
    let store = store(&endpoint);
    let ns = namespace("Blastlands");
    let mut expected = 0;

    for size in [1usize, 7, 40, 300, 2, 91, 5000, 13] {
        let payload = vec![b'a'; size];
        let oid = hex::encode(sha2::Sha256::digest(&payload));
        let (_root, path) = staged(&payload).await;

        store.store(&ns, &oid, &path).await.unwrap();
        expected += size as u64;
    }

    assert_eq!(store.usage_of(&ns).await, (8, expected));
}
