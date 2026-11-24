<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/assets/brand/wordmark.dark.svg">
    <img alt="lemon8s" src="docs/assets/brand/wordmark.light.svg" width="440">
  </picture>
</p>

<h3 align="center">Refreshingly orchestrated local AI</h3>

One private AI endpoint for your team, with the easy deployment of Lemonade and the controls and scale of Kubernetes.

> [!WARNING]
> lemon8s is an experimental prototype. Its code arrives in this repository one part at a time, each with its tests and docs, and none of it is here yet. Expect breaking changes, and don't rely on lemon8s to protect sensitive data.

lemon8s runs [Lemonade](https://github.com/lemonade-sdk/lemonade)'s llama.cpp builds on [K3s](https://k3s.io), a lightweight Kubernetes, behind a gateway of its own, on Linux hosts with AMD GPUs. People and their AI agents connect with any OpenAI-compatible app. Each conversation is tagged with the kinds of data it may hold, and only people cleared for those tags can open it. To add capacity, you add a host, and the same URL serves more requests.

## What's here so far

- [ ] **Policy:** conversation tags, clearances, and who may open what.
- [ ] **Engines:** Lemonade's llama.cpp builds, locked down, with no network access.
- [ ] **Gateway and control service:** the OpenAI-compatible endpoint, with a key and a KV cache per conversation.
- [ ] **One host:** the Helm chart, and the DRA driver that gives each engine its GPU, for one K3s host.
- [ ] **The `lemon8s` command:** install, offline use and status.
- [ ] **More hosts:** join a host, and choose which tags it may serve.
- [ ] **Measurements and design docs.**

## Contributing

Questions and ideas are welcome in an issue or on [Lemonade's Discord](https://discord.gg/5xXzkMu8Zk). See [CONTRIBUTING.md](.github/CONTRIBUTING.md), report vulnerabilities as [SECURITY.md](.github/SECURITY.md) describes, and follow the [Code of Conduct](.github/CODE_OF_CONDUCT.md).

## License

lemon8s is an open-source project supported by AMD, licensed under the [Apache License 2.0](LICENSE).
