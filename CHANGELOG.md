# Changelog

All notable changes to the Will contract are documented here.

## [1] - Contract versioning

- Added `VERSION` constant to the Will contract.
- Added `get_contract_version` read-only query entrypoint.
- Added `get_contract_version_symbol` for human-readable version display.
- Version is now explicit and queryable so multiple deployed versions can be distinguished.
