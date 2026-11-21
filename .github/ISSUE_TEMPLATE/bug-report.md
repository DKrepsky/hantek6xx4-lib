---
name: Bug report
about: Something does not work as it should
title: ''
labels: bug
assignees: ''
---

# Description

<!--
One or two sentences: what happens.
Mention the device involved (model, and the PCB revision and FPGA version printed by `hantek6xx4 info`) and how often it happens (always, sometimes, once).
-->

# Steps to reproduce

<!-- Start from a known state (scope plugged in or not, nothing else using it). One action per step: the CLI command or the library calls, with their arguments. -->

1.
2.
3.

# Expected behavior

<!-- What should have happened instead. If you are not sure, describe what you think would be right and why. -->

# Evidence

<!--
Log output (run with `RUST_LOG=debug` and paste the relevant lines in a code block), error text, captured samples or
`hantek6xx4 raw` output, and what the signal on the input was (calibrator, DC level, other).
Add the crate or CLI version (or commit), the Rust version, the Linux distribution and kernel messages
(`dmesg`) if the device disappears or fails to enumerate.
-->
