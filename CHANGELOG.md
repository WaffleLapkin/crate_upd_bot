# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [unreleased]

## 0.1.9

### Fixed

- Panics on failed git pull (remove `.expect`s)

## 0.1.8

### Changed

- Improve some panic messages for easier problem identifying
- Remove `dbg!` exprs

## 0.1.7

### Added

- Unsubscribe users from all crates when they block the bot

## 0.1.6

### Changed

- Prepare database queries beforehand

### Fixed

- Spirious timeout errors (move git work onto non-tokio thread)
- `^C` handling (shutdown instead of hanging)

## 0.1.5

### Changed

- Internal rewrite (move to `teloxide`)

## 0.1.3

### Added

- List of banned crates to prevent spamming by releasing new versions

## 0.1.2

### Fixed

- Use index path from config, when reading last crate version 

## 0.1.1

### Added

- Include current version in `/start` output

### Changed

- All commits in the `crates.io-index` which author is not `bors` are now ignored. Earlier those commits were causing 
  crashes (panics).
