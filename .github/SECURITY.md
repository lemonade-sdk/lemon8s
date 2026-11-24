# Security policy

lemon8s aims to keep each conversation private to the people, agents, and hosts allowed to see it, so any way around that is a vulnerability, and we want to hear about it.

## Report a vulnerability

Please report vulnerabilities privately. Don't open a public issue, discussion, or pull request: that discloses the problem before a fix exists.

On this repository's **Security** tab, choose **Report a vulnerability**. This opens a private advisory that only you and the maintainers can see.

Include what you can:

- the commit you tested (`git rev-parse HEAD`)
- your hardware
- what an attacker gains
- steps to reproduce, or a minimal proof of concept
- a fix or mitigation, if you have one

We'll acknowledge your report as soon as we can and keep you updated while we fix it.

## Supported versions

lemon8s is a prototype and has no releases yet. Only the `main` branch is supported: fixes land there, and earlier commits aren't patched.

## Out of scope

- **Upstream components lemon8s runs unmodified**, such as K3s, llama.cpp, and Lemonade: report those to their projects. If lemon8s's configuration of a component is what makes it exploitable, report it here.
- **AMD hardware, drivers, and firmware:** report through [AMD Product Security](https://www.amd.com/en/resources/product-security.html).
