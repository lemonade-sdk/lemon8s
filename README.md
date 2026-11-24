<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/assets/brand/wordmark.dark.svg">
    <img alt="lemon8s" src="docs/assets/brand/wordmark.light.svg" width="440">
  </picture>
</p>

<h3 align="center">Refreshingly orchestrated local AI</h3>

Goal: one private AI endpoint for your team, with the easy deployment of Lemonade and the controls and scale of Kubernetes.

> [!WARNING]
> lemon8s is an experimental prototype. Expect breaking changes, and don't rely on lemon8s.

lemon8s runs [Lemonade](https://github.com/lemonade-sdk/lemonade) using [K3s](https://k3s.io), a lightweight Kubernetes. People and their AI agents connect with any OpenAI-compatible app. Each conversation is tagged with the kinds of data it may hold, and only people cleared for those tags can open it. To add capacity, you add a host, and the same URL serves more requests.

## Contributing

Questions and ideas are welcome in an issue or on [Lemonade's Discord](https://discord.gg/5xXzkMu8Zk). See [CONTRIBUTING.md](.github/CONTRIBUTING.md), report vulnerabilities as [SECURITY.md](.github/SECURITY.md) describes, and follow the [Code of Conduct](.github/CODE_OF_CONDUCT.md).

## License

lemon8s is an open-source project, licensed under the [Apache License 2.0](LICENSE).
