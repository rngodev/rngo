# Migrating specs

A spec declares the format it uses with a top-level `version`. A spec with no `version` is version 1. `rngo` reads one version at a time and says so when a spec declares another.

Editors can validate and autocomplete specs with the JSON Schemas in `schema/v1/`. `rngo init` adds the `$schema` line to a new `spec.yml`.

## Version 1

The first versioned format. Specs written before versioning were already in it if they use `prototypes`, `generators` and flat `channels`. If `rngo` reports a key from before versioning, update it:

- `phases`: replaced by `prototypes` (named defaults an effect opts into with `prototype`); a prototype takes `start`, `end`, `limit` and `channel`, not lists of effects or channels.
- `schemas`: renamed to `generators`.
- a channel's `target`: removed; put the target's fields (`type`, `command`, ...) directly on the channel next to `format`.
