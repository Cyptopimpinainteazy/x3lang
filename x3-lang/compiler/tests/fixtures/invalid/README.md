# Invalid fixtures

Files here are **meant** to be refused, and they are fixtures rather than programs: each one
is `include_str!`d by a test that asserts the refusal and, where the refusal has a code, the
message.

The corpus gate (`every_x3_file_the_tooling_walks_is_a_program` in
`crates/x3-tools/tests/cli.rs`) walks every `.x3` file outside its named non-language
directories and requires `x3c check` to pass. This directory is skipped **by name**, which is
why a file that is supposed to fail lives under a directory that says so rather than beside
the ones that are supposed to pass.
