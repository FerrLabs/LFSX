use crate::error::Error;

// The digest a client names an object by, validated once, where it enters.
// Everything downstream takes the type and cannot be handed anything else,
// the same shape `Namespace` already has: there is no unvalidated `Oid` to
// pass, so there is nothing for the eighteenth call site to forget.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Oid(String);

impl Oid {
    pub fn parse(raw: &str) -> Result<Self, Error> {
        let well_formed = raw.len() == 64
            && raw
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));

        well_formed
            .then(|| Self(raw.to_owned()))
            .ok_or(Error::MalformedOid)
    }

    // The two directory levels an object files under, on disk and in a bucket
    // alike. Safe to slice because parsing guaranteed sixty-four ASCII bytes,
    // which is the fact this type exists to carry.
    pub fn fanout(&self) -> (&str, &str) {
        (&self.0[0..2], &self.0[2..4])
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Oid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_lowercase_hex_sha256_digest_parses() {
        let digest = "a".repeat(64);

        assert_eq!(Oid::parse(&digest).unwrap().as_str(), digest);

        for raw in [
            "",
            "abc",
            &"A".repeat(64),
            &"g".repeat(64),
            &"a".repeat(63),
            &"a".repeat(65),
            &format!("../{}", "a".repeat(61)),
        ] {
            assert!(
                matches!(Oid::parse(raw), Err(Error::MalformedOid)),
                "{raw:?} must not parse"
            );
        }
    }

    #[test]
    fn the_fanout_is_the_first_four_characters() {
        let oid = Oid::parse(&format!("abcd{}", "0".repeat(60))).unwrap();

        assert_eq!(oid.fanout(), ("ab", "cd"));
    }
}
