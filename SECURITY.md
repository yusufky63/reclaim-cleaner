# Security policy

ReClaim deletes files and can run with administrator rights, so security reports get priority.

## Supported versions

Only the latest release receives fixes.

## Reporting a vulnerability

Please do not open a public issue. Use GitHub's private reporting: **Security > Report a vulnerability** on this repository. Include the version, Windows build, steps to reproduce and what an attacker gains.

You can expect a first answer within a week. Fixed issues are credited in the release notes unless you ask otherwise.

## What counts

Examples that are in scope:

- A way to make the app or its elevated helper delete files outside the locations it shows (for example through junctions, crafted rule files, the helper request file or path tricks).
- Protected locations or user exclusions being bypassed.
- Online-only cloud files being downloaded or hashed.
- Any network traffic other than the opt-in update check.

Bugs in which the user deliberately selects and confirms their own files are not vulnerabilities, but please still report them as bugs.
