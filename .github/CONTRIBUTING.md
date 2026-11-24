# Contributing to lemon8s

Thanks for your interest in lemon8s! It's an experimental prototype at present, and its interfaces, structure, and even goals are subject to change. We welcome community discussions.

Everyone taking part must follow the [Code of Conduct](CODE_OF_CONDUCT.md). Report vulnerabilities privately, as [SECURITY.md](SECURITY.md) describes.

## Issues and pull requests

Ask questions and share ideas in an issue or on [Lemonade's Discord](https://discord.gg/5xXzkMu8Zk). Small fixes can go straight to a pull request. Anything bigger, such as a new component or dependency, starts with an issue, so the approach is agreed before you write the code.

## How we build lemon8s

lemon8s follows [Lemonade's philosophy](https://github.com/lemonade-sdk/lemonade/blob/main/docs/dev/philosophy.md). In particular, if Kubernetes, K3s, Lemonade, or a maintained upstream tool already does the job, we use it instead of writing our own. We aim for wide compatibility over a strictly optimized configuration.

## Tests

Every pull request runs [the checks](workflows/check.yml): spelling, Markdown, and links; Python lint and format; the chart's lint, schema, and [unit tests](../deploy/chart/lemon8s/tests); the workflows themselves; and Lemonade's [comment check](https://github.com/lemonade-sdk/lemonade/blob/main/tools/check_comment_slop.py) on the lines a pull request changes.

The end-to-end tests in [`tests/e2e`](../tests/e2e) run against a live install, through the port-forward and variables from the README's step 3. They unload every model, so use a test install, not one serving people. They check that the API key is required and can't change Lemonade's settings, that the model runs with the backend you name, that a model on the GPU takes GPU memory, and that the model answers a question:

```bash
python3 -m venv .venv
.venv/bin/pip install --require-hashes -r tests/requirements.txt
.venv/bin/pytest tests/e2e --backend vulkan
```

Name the backend you set. On a Strix Halo, `auto` picks `vulkan`. A pull request that changes the chart says which backends and hardware the end-to-end tests passed on.

## AI tools

lemon8s follows [Lemonade's AI content policy](https://github.com/lemonade-sdk/lemonade/blob/main/docs/dev/ai-content-policy.md). Write your issues, discussions, pull request descriptions, comments, and Discord messages yourself. You may use AI tools for code and reviews, but you're responsible for what you submit: understand it, review the full diff yourself, remove unrelated changes, test it locally, and credit the tool with a `Co-Authored-By:` trailer.

## License

What you contribute is licensed under [Apache-2.0](../LICENSE), as section 5 of the license sets out. There's no sign-off or contributor agreement, and new files need no license header.
