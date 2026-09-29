# Benchmark methodology

Glasir's benchmark is a regression test for this repository. It is not a
general performance claim, a model evaluation, or a substitute for evaluating
Glasir on a customer's source tree.

## What is measured

`glasir benchmark <tree>` loads question sets from `bench/`. Each question
lists the symbols an answer must reach. The command reports recall, returned
items, precision, and an approximate token cost for two deterministic methods:

1. **Glasir** uses the same bounded graph-query path exposed to a client.
2. **grep + read** tokenizes the question with the same identifier treatment,
   ranks source files by matching terms, opens at most five files, and receives
   credit for expected symbols defined in those files.

The baseline is deliberately stronger than a literal `grep -r` invocation, but
it cannot traverse graph edges. It is a local comparison of two retrieval
strategies, not a claim about any particular agent or model.

Token cost is source characters divided by four. It is an approximation applied
equally to both methods; it must not be interpreted as a provider billing value.

## Reproducing a result

Run the command from the exact commit and on the exact corpus to be reported:

```sh
cargo build --release --locked
./target/release/glasir benchmark .
./target/release/glasir benchmark . --check
```

`--check` compares Glasir recall with floors in `bench/baseline.txt`. Those
floors guard against regressions; they are not product targets and do not make
a claim about unrelated repositories. Record the release version, commit,
hardware, operating system, corpus revision, command output, and any parser or
language-rule configuration when publishing an external result.

## Limits

Question sets are curated and small. They can reveal regressions in known
retrieval and structural cases, but they do not measure all languages, runtime
behaviour, security impact, or customer-specific architecture. Validate impact
and path results against representative repositories before relying on them in
a production workflow.
