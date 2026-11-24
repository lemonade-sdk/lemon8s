# Security policy

lemon8s exists to keep each conversation private to the people and hosts allowed to see it, so any way around that is a vulnerability, and we want to hear about it.

## Report a vulnerability

Please report vulnerabilities privately. Don't open a public issue, discussion, or pull request: that discloses the problem before a fix exists.

Use either channel:

- **GitHub:** on this repository's **Security** tab, choose **Report a vulnerability**. This opens a private advisory that only you and the maintainers can see.
- **AMD Product Security:** follow the process on [AMD's product security page](https://www.amd.com/en/resources/product-security.html).

Include what you can:

- the commit you tested (`git rev-parse HEAD`)
- your hardware
- what an attacker gains
- steps to reproduce, or a minimal proof of concept
- a fix or mitigation, if you have one

We'll acknowledge your report as soon as we can, keep you updated while we fix it, agree a disclosure date with you, and credit you in the advisory unless you'd rather not be named.

## Supported versions

lemon8s is a prototype and has no releases yet. Only the `main` branch is supported: fixes land there, and earlier commits aren't patched.

## What counts as a vulnerability

Today lemon8s runs one Lemonade server, and it promises three things: only its host can reach it, every request but Lemonade's liveness check needs the API key, and the API key can't change Lemonade's settings. A way around any of them is a vulnerability.

lemon8s's other promises arrive with the parts that keep them. Until a part is here, a flaw in its design is welcome as an ordinary issue.

## Out of scope

- **Upstream components lemon8s runs unmodified**, such as K3s, llama.cpp, and Lemonade: report those to their projects. If lemon8s's configuration of a component is what makes it exploitable, report it here.
- **AMD hardware, drivers, and firmware:** report through [AMD Product Security](https://www.amd.com/en/resources/product-security.html).
