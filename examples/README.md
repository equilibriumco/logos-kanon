# `examples/`

Runnable examples for the developer journeys the doc packets describe: pushing a
package, reading the canonical account, and verifying a payload in public-mode
pull.

Empty by design at M1. Examples are written against a public API, and the SDK's
API lands in M4-11; writing them earlier would mean writing them twice. The doc
packets (M5-04, M5-05) reference what lands here, so an example that stops
compiling is a documentation defect and CI treats it as one.

Distinct from `reference-consumers/`, which is a deliverable rather than
illustration: those are the two consumer *programs* the RFP asks for (Usability
7), each with its own tests and each demonstrating the recommended integration
pattern for one mode.
