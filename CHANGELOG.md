# Changelog

All notable changes to this project will be documented in this file.

## [1.0.0] - 2025-07-30

### Added
- Configurable listen durations via `IDLE_LISTEN_SECS` and `CONVO_LISTEN_SECS` env vars (default 2s and 5s).
- Package metadata in Cargo.toml (`description`, `license`, `readme`, `keywords`).
- Version bump to 1.0.0.

### Changed
- Reduced default idle listen duration from 5s to 2s for faster wake word detection.
- Reduced default conversation listen duration from 10s to 5s for improved responsiveness.
- Enhanced startup logging with configured listen durations.
- Updated README and documentation for a production-ready v1.0.0 release.
