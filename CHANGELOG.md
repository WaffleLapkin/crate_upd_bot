# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## 0.1.1

### Changed

- All commits in the `crates.io-index` which author is not `bors` are now ignored. Earlier those commits were causing 
  crashes (panics).
