# Contributing

Bug reports and patches are welcome. This file says what happens to them and what a change needs to
carry before it can be merged, so that nothing about the process is a surprise.

## Reporting a bug

Open an [issue](https://github.com/FerrLabs/LFSX/issues). What makes a report actionable, in rough
order of usefulness:

- The version (`lfsx-server --version`) and how it is deployed: volume or bucket, which auth
  provider, behind what proxy.
- What the client did and what it saw. The output of `git lfs push` with `GIT_TRACE=1` and
  `GIT_CURL_VERBOSE=1` usually contains the whole story.
- The server's logs around the same moment, at `RUST_LOG=debug` if the default says nothing.
- What you expected instead. Sometimes the disagreement is about the protocol rather than the code,
  and that is worth knowing early.

Never paste a token. The traces above contain the `Authorization` header.

For anything with a security consequence, read [SECURITY.md](SECURITY.md) instead: those go through
a private advisory rather than an issue.

## Proposing a change

Open an issue before a large change. Not as a formality: this server refuses features on purpose,
and it is better to hear that a direction is out of scope before writing it than after. Small fixes
can go straight to a pull request.

A pull request for anything larger references its issue, and the branch is named for what it does
(`feat/bucket-cache`, `fix/lock-takeover`).

## What a change has to carry

**Tests.** A fix carries the test that fails without it. A feature carries the tests for its edge
cases and its error paths, not only the happy one. A test earns its place by failing when the
behaviour it guards breaks: if you cannot say which bug it would catch, it is noise rather than
coverage.

**A green gate.** `cargo test`, `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check`
all pass. The bucket suite runs against MinIO and Garage in CI, so a storage change is exercised
against two real implementations rather than a stub alone.

**Documentation, in the same change.** A new environment variable belongs in
[`docs/configuration.md`](docs/configuration.md), a new behaviour in the page that covers it, and a
user-visible change on the marketing site. Docs that trail behind the code are worse than missing
docs, because they are believed.

**Conventional commits.** `type(scope): description`, in the imperative, one line. The title of a
pull request becomes the commit on `main` and drives the version bump, so `feat:` and `fix:` are
load-bearing. Reserve `!` for a genuine breaking change: a removed flag, a renamed field, a format
that rejects what it used to accept. Adding something is not breaking.

**Comments that earn their place.** The code here explains itself through names and structure; a
comment is for the reason a reader could not recover from the code, usually why an obvious approach
was rejected. See the existing source for the register.

## What happens next

A pull request is reviewed, and the review takes the work seriously enough to disagree with it.
Expect questions about the failure mode you did not mention and the test that would have caught it.
Nothing merges without a green CI and a review.

## Building it

[`docs/development.md`](docs/development.md) has the commands, what each suite actually exercises,
and how to run the coverage report and the fuzz targets.

The one thing that page does not cover is the bucket suite, which needs an S3-compatible store to
point at:

```bash
docker run -d --name minio -p 9000:9000 \
  -e MINIO_ROOT_USER=lfsxkey -e MINIO_ROOT_PASSWORD=lfsxsecret \
  quay.io/minio/minio server /data

# once http://127.0.0.1:9000/minio/health/live answers
docker exec minio mc alias set local http://127.0.0.1:9000 lfsxkey lfsxsecret
docker exec minio mc mb --ignore-existing local/lfsx-test

LFSX_TEST_S3_ENDPOINT=http://127.0.0.1:9000 cargo test --test bucket
```

The bucket has to exist before the suite runs: nothing in the server creates one, and
`an_instance_that_cannot_reach_its_bucket_is_not_ready` depends on a missing bucket staying missing.
The name comes from `LFSX_TEST_S3_BUCKET`, which defaults to `lfsx-test`.

Without `LFSX_TEST_S3_ENDPOINT` those tests skip rather than fail, so a plain `cargo test` stays
useful on a laptop with nothing installed.

## Licence

Contributions are licensed under the [MPL-2.0](LICENSE), the licence this project ships under.
