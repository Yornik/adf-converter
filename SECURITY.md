# Security policy

## Supported versions

Only the [latest release](https://github.com/Yornik/adf-converter/releases/latest) gets security
fixes. Please check that a problem still happens with it before reporting.

## Reporting a vulnerability

Report vulnerabilities **privately** through GitHub's private vulnerability reporting: open the
repository's **Security** tab and choose **Report a vulnerability**
(<https://github.com/Yornik/adf-converter/security/advisories/new>). Please don't open a public
issue, discussion or pull request for a vulnerability. If that button is not available, open an
issue that only asks for a private contact, without any details of the problem.

Include, if you can:

- the output of `adf version --json`;
- the exact command and a minimal input that shows the problem (invented content only, never real
  Jira or Confluence data);
- what happens and what you expected, and the impact as you see it.

This is a free-time project maintained by one person, so there is no guaranteed response time. I
will acknowledge the report as soon as I can, keep you informed, and credit you in the advisory
unless you prefer not to be named.

## Scope

`adf` is an offline converter. It makes no network requests, does not handle credentials, reads
no configuration files, and reads only the environment variables listed in
[docs/cli-contract.md](docs/cli-contract.md). In scope is how it handles **untrusted input**,
which is the normal case: AI agents and scripts generate the Markdown and ADF it reads.

In scope:

- crashes, panics, hangs, stack overflows or runaway memory use on any input, including input
  that gets past the size and nesting limits;
- output that is unsafe although the input was converted "successfully", for example a
  `javascript:` or other non-allowlisted URL that ends up as a link, or content from one part of
  the input leaking into another;
- writing to a path other than the one given with `-o`, or leaving a partial file behind;
- problems in the release process: tampered or unverifiable release archives, checksums or
  attestations, and weaknesses in this repository's GitHub Actions workflows.

Out of scope:

- vulnerabilities in **Jira, Confluence or other Atlassian products**, including how they render
  ADF. Report those to Atlassian:
  <https://www.atlassian.com/trust/security/report-a-vulnerability>. This project is not
  affiliated with Atlassian.
- the `curl`, acli and other commands in the docs that *you* run to post content, and the
  credentials you use with them;
- bugs without a security impact, such as wrong formatting or a missing warning; please open a
  normal [issue](https://github.com/Yornik/adf-converter/issues) for those.
