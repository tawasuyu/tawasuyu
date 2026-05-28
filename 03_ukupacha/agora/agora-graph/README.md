# agora-graph

> Trust graph of [agora](../README.md): verified attestations + corroboration + negotiated policy.

A `TrustGraph` stores known identities and **verified** attestations: `add_attestation` runs `Attestation::verify` before accepting, so a broken signature can never enter. Duplicates are dropped silently — convergence is idempotent.

The graph deliberately does **not** emit a verdict. `corroboration(subject, predicate, value)` returns the raw evidence: distinct attesters and whether the subject self-attested. `TrustPolicy { min_third_party, accept_self }` is the *negotiated* threshold each reader adopts. Two readers with different policies looking at the same graph may legitimately disagree.

## Deps

- [`agora-core`](../agora-core/README.md), `serde`
