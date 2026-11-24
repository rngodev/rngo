# Migrating

## Specs

A spec declares the format it uses with a top-level `version`. A spec with no `version` is version 1. `rngo` reads one version at a time and says so when a spec declares another.

Editors can validate and autocomplete specs with the JSON Schemas in `schema/v1/`. `rngo init` adds the `$schema` line to a new `spec.yml`.

### Version 1

The first versioned format. Specs written before versioning were already in it if they use `prototypes`, `generators` and flat `channels`. If `rngo` reports a key from before versioning, update it:

- `phases`: replaced by `prototypes` (named defaults an effect opts into with `prototype`); a prototype takes `start`, `end`, `limit` and `channel`, not lists of effects or channels.
- `schemas`: renamed to `generators`.
- a channel's `target`: removed; put the target's fields (`type`, `command`, ...) directly on the channel next to `format`.

An unknown field on an effect, a trigger object, a prototype's scenario or a custom generator is rejected rather than ignored. Such a field never did anything, so remove it, or fix the typo it hides (`limt` for `limit`, `chanel` for `channel`).

## Log

A log (`log.sqlite`) records the version of its table layout, which [LOG.md](LOG.md) documents. `rngo` opens only logs of the version it writes, and reports a log of another version, or one written before logs were versioned, without changing it. SQL signals written against one version may need updating for the next.

### Version 1

The first versioned layout: the `inputs`, `outputs` and `metadata` tables as written by rngo 0.40, now stamped with `PRAGMA user_version = 1`. Logs written before versioning can still be read directly with SQLite, but `SqliteLog::in_dir` won't reopen them; start a new log instead.
