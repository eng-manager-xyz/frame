# Studio contract fixtures

`cap-schema-supported/` is a locally authored `.cap` directory fixture shaped
after the schema at
`CapSoftware/Cap@6ba69561ac86b8efdb17616d6727f9638015546b`. It contains:

- `recording-meta.json` with two sequential recording segments;
- `project-config.json` with supported timeline operations; and
- segment-local display, camera, microphone, and system-audio descriptors under
  `content/segments/segment-N/`.

The field and path spellings are pinned from that revision's
`crates/project/src/meta.rs`, `crates/project/src/configuration.rs`, and
`crates/recording/src/studio_recording.rs` definitions. The fixture selects the
non-fragmented `.mp4`/`.ogg` variant.

The small `.mp4` and `.ogg` files contain deterministic descriptor bytes. They
are not valid encoded media and are used only to establish byte lengths,
checksums, paths, segment/track identity, read-only source fingerprinting, and
the production filesystem adapter's explicit source-to-destination copy plan.
The tests parse the JSON through `FilesystemLegacyCapProjectPort`; malformed
JSON, traversal, missing media, and symlink fixtures fail closed. Unsupported
effect/newer-version reporting is still contract evidence, not historical
product parity. A test also rewrites this source fixture into the pinned
flattened single-segment metadata form and adds the known empty/default
timeline and audio fields, proving that both upstream schema forms pass through
the same typed adapter.

These files are synthetic contract fixtures, not historical-product or media
quality evidence. A real-world Cap compatibility corpus remains protected
evidence because it requires provenance, privacy review, a frozen reference
revision, and approved expected outcomes.
