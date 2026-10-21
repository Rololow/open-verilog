# Signing ovc.exe and the installer

Unsigned binaries make Windows SmartScreen warn the user. The free way to sign them is the **SignPath Foundation**
program for open source projects. Nothing is paid.

## Prerequisites (checked by SignPath)
- a public repository under an OSI-approved licence (MIT: yes, see `LICENSE`);
- the binaries are built by a CI workflow in that repository (`.github/workflows/release.yml`);
- the maintainer's GitHub account has two-factor authentication enabled.

## Steps
1. Apply at https://signpath.org/foundation (the form asks for the project, its maintainers and its links).
2. When approved, SignPath gives an organization ID, a project slug, a signing policy and an API token.
3. Store them in the GitHub repository: the token as the secret `SIGNPATH_API_TOKEN`, the other values as variables.
4. Add the signing steps to `release.yml`: after the binaries are built, submit them with the SignPath GitHub
   action and replace them with the signed files; then do the same for the installer after `ISCC` has built it.
   The exact inputs come from the SignPath documentation of the approved project.

## Until then
The release is published unsigned, with its SHA256SUMS. The VS Code extension verifies the checksums, so an
unsigned download is still checked.
