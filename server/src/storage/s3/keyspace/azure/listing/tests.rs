use super::*;

const PAGE: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<EnumerationResults ServiceEndpoint="https://account.blob.core.windows.net/" ContainerName="assets">
  <Prefix>acme/</Prefix>
  <Blobs>
    <Blob>
      <Name>acme/game/ab/cd/abcd</Name>
      <Properties>
        <Creation-Time>Wed, 30 Sep 2026 10:00:00 GMT</Creation-Time>
        <Last-Modified>Wed, 30 Sep 2026 10:04:05 GMT</Last-Modified>
        <Etag>0x8D</Etag>
        <Content-Length>1234</Content-Length>
        <BlobType>BlockBlob</BlobType>
      </Properties>
      <OrMetadata />
    </Blob>
    <Blob>
      <Name>acme/game/ef/01/ef01</Name>
      <Properties>
        <Last-Modified>not a date</Last-Modified>
        <Content-Length>0</Content-Length>
      </Properties>
    </Blob>
  </Blobs>
  <NextMarker>2!80!MDAwMDE2</NextMarker>
</EnumerationResults>"#;

#[test]
fn a_page_names_every_blob_with_its_size_and_when_it_was_written() {
    let page = parse(PAGE).unwrap();
    let written = time::Date::from_calendar_date(2026, time::Month::September, 30)
        .unwrap()
        .with_hms(10, 4, 5)
        .unwrap()
        .assume_utc();

    assert_eq!(page.entries.len(), 2);
    assert_eq!(page.entries[0].key, "acme/game/ab/cd/abcd");
    assert_eq!(page.entries[0].size, 1234);
    assert_eq!(page.entries[0].modified, Some(written));
    assert_eq!(page.next.as_deref(), Some("2!80!MDAwMDE2"));
}

#[test]
fn an_unreadable_date_is_unknown_rather_than_old() {
    let page = parse(PAGE).unwrap();

    assert_eq!(
        page.entries[1].modified, None,
        "a date this server cannot read is treated as too young to sweep, never as ancient"
    );
}

#[test]
fn the_last_page_carries_an_empty_marker() {
    let last = PAGE.replace("<NextMarker>2!80!MDAwMDE2</NextMarker>", "<NextMarker />");

    assert_eq!(parse(&last).unwrap().next, None);
}

#[test]
fn an_empty_container_lists_nothing() {
    let empty = r#"<?xml version="1.0" encoding="utf-8"?><EnumerationResults><Blobs /><NextMarker /></EnumerationResults>"#;

    let page = parse(empty).unwrap();

    assert!(page.entries.is_empty());
    assert_eq!(page.next, None);
}
