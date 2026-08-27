"""The SDK's own SemVer.

Independent of the API contract by design: a package version and an API version
answer different questions. `temper.CONTRACT_VERSION` is the other one.

This file is READ (never imported) by hatchling at build time, so it must stand
alone: no import of anything under `temper`, and in particular nothing from
`temper.generated`, which is not installed while the wheel is being built.
"""

__version__ = "0.1.0"
