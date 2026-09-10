# ExpOS embedded-tls patch

This directory vendors `embedded-tls` 0.19.0 under its original Apache-2.0
license. ExpOS changes one fixed-capacity constant in `src/der_certificate.rs`:
the retained DNS Subject Alternative Name count is 96 instead of 3.

The upstream three-name limit rejects valid production certificates when the
requested hostname appears later in the SAN extension. Google's current leaf
certificate places YouTube names after more than fifty entries. The patch does
not change trust anchors, chain verification, signature verification,
certificate dates, SNI, or hostname matching.

`make https-check` exercises the patched verifier against
`https://www.youtube.com/` and fails if authenticated TLS or HTTP does not
complete.
