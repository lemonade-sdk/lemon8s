# Contributing to lemon8s

Thanks for your interest in lemon8s! It's an experimental prototype, arriving in this repository one part at a time, and its interfaces still change.

Everyone taking part follows the [Code of Conduct](CODE_OF_CONDUCT.md). Report vulnerabilities privately, as [SECURITY.md](SECURITY.md) describes, never in an issue.

## Issues and pull requests

Ask questions and share ideas in an issue or on [Lemonade's Discord](https://discord.gg/5xXzkMu8Zk). Small fixes can go straight to a pull request. Anything bigger, such as a new component or dependency, starts with an issue, so the approach is agreed before you write the code.

## How we build lemon8s

lemon8s follows [Lemonade's philosophy](https://github.com/lemonade-sdk/lemonade/blob/main/docs/dev/philosophy.md). On top of it: if Kubernetes, K3s, Lemonade or a maintained upstream tool already does the job, we use it instead of writing our own; code with no caller is removed until something needs it; and a feature is described as working only once it does.

## Commits

Each commit is one logical change. Its subject is an imperative sentence in plain words, with no type prefix and no trailing period. Its body says why the change is needed, what it does and what you checked.

You're welcome to use AI coding tools, under [Lemonade's AI content policy](https://github.com/lemonade-sdk/lemonade/blob/main/docs/dev/ai-content-policy.md): write issues and pull request descriptions yourself, understand and review everything you submit, and credit the tool with a `Co-Authored-By:` trailer.

## License

What you contribute is licensed under [Apache-2.0](../LICENSE), as section 5 of the license sets out. There's no sign-off or contributor agreement, and new files need no license header.

## Maintainers

| Maintainer | Areas |
|---|---|
| [@hunhoffe](https://github.com/hunhoffe) | All |
