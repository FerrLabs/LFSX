use super::*;

fn parse(args: &[&str]) -> Command {
    Cli::parse_from(
        ["lfsx", "--url", "http://localhost"]
            .into_iter()
            .chain(args.iter().copied()),
    )
    .command
}

#[test]
fn whole_repository_commands_are_not_cut_off() {
    for args in [
        &["gc", "--repo", "o/r"][..],
        &["dedupe", "--repo", "o/r"],
        &["compress", "--repo", "o/r"],
        &["verify", "--repo", "o/r"],
    ] {
        assert_eq!(parse(args).request_timeout(), None, "{args:?}");
    }
}

#[test]
fn doctor_still_gives_up_on_a_silent_server() {
    assert_eq!(parse(&["doctor"]).request_timeout(), Some(DOCTOR_TIMEOUT));
}
