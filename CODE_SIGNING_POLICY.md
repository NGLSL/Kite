# Kite Code signing policy

Kite is preparing an application for free code signing through SignPath.io. The application has not been approved, and no current Kite release is signed by SignPath Foundation. The following credit applies only if the application is approved and the published files are actually signed:

> Free code signing provided by SignPath.io, certificate by SignPath Foundation.

The source repository is [NGLSL/Kite](https://github.com/NGLSL/Kite). Release downloads are published on [GitHub Releases](https://github.com/NGLSL/Kite/releases). Check a downloaded file's digital signature before treating it as signed; a policy page alone does not authenticate a file.

## Project roles

- Committer and reviewer: [NGLSL](https://github.com/NGLSL)
- Release signing approver: [NGLSL](https://github.com/NGLSL)

Changes from other contributors require review by the maintainer before inclusion. Every signing request will require the designated approver's approval. Signing will apply only to release artifacts built from this repository after the signing service has approved the project.

## Privacy and network activity

Kite keeps its app index, settings, launch history, and diagnostic log on the user's computer. Kite does not include telemetry or analytics reporting.

- When a user clicks **Check for updates**, Kite requests release information from the GitHub API. When the user chooses to download an update, Kite downloads the installer from GitHub Releases.
- When a user invokes web search, the search query is sent to the search provider configured on that computer.
- Optional Everything searches use local IPC with an installed and running Everything instance.

Kite does not send the local app index, launch history, or diagnostic log to Kite's maintainers. External sites and applications opened by the user have their own network behavior and privacy policies.
