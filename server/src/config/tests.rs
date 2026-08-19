use super::*;

// The default is the whole point of this function: an operator who sets nothing
// gets a server that asks for a credential. Everything else here guards against
// a value that looks like consent without being it.
#[test]
fn anonymous_read_is_closed_unless_it_is_asked_for() {
    assert!(!anonymous_read(None), "unset must not open it");

    for refused in ["false", "", "1", "yes", "True", "TRUE", " true", "true "] {
        assert!(
            !anonymous_read(Some(refused)),
            "{refused:?} is not the string that opens it"
        );
    }

    assert!(anonymous_read(Some("true")), "the one value that opens it");
}
