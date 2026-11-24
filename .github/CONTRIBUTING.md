# Contributing to lemon8s

Thanks for your interest in lemon8s! It's an experimental prototype at present, and its interfaces, structure, and even goals are subject to change. We welcome community interest and discussion.

Everyone taking part must follow the [Code of Conduct](CODE_OF_CONDUCT.md). Report vulnerabilities privately, as [SECURITY.md](SECURITY.md) describes.

## Issues and pull requests

Ask questions and share ideas in an issue or on [Lemonade's Discord](https://discord.gg/5xXzkMu8Zk). Small fixes can go straight to a pull request. Anything bigger, such as a new component or dependency, starts with an issue, so the approach is agreed before you write the code.

## How we build lemon8s

lemon8s follows [Lemonade's philosophy](https://github.com/lemonade-sdk/lemonade/blob/main/docs/dev/philosophy.md). In particular, if Kubernetes, K3s, Lemonade, or a maintained upstream tool already does the job, we use it instead of writing our own. We aim for wide compatibility over a strictly optimized configuration.

Before you ask for review, check your change the way reviewers will:

- **Nothing reinvented,** as above.
- **Nothing extra:** can anything be deleted or merged with no loss?
- **Said once:** is this logic, setting, or explanation already somewhere else?
- **Safe by default:** does it widen what anyone can reach, or weaken a default?
- **Holds up on a shared cluster:** upgrades, restarts, eviction, a full disk.
- **Clear to the person using it:** someone installing for the first time, or connecting an app.

## Tests

Every pull request runs [the checks](workflows/check.yml): spelling, Markdown, and links; Python lint and format; the chart's lint, schema, and [unit tests](../deploy/chart/lemon8s/tests); the workflows themselves; Lemonade's [comment check](https://github.com/lemonade-sdk/lemonade/blob/main/tools/check_comment_slop.py) on the lines a pull request changes; and the end-to-end tests below, on the CPU, in K3s on Linux. The **Checks** job passes only when all of them do. A Windows job runs the same end-to-end tests in [Rancher Desktop](https://rancherdesktop.io) but doesn't block a merge yet.

The end-to-end tests in [`tests/e2e`](../tests/e2e) can be run against a live install, through the port-forward and variables from the README's step 3. The tests unload every model, so use a test install. The tests check that the API key is required and can't change Lemonade's settings, that the model runs with the backend you name, that a model on the GPU takes GPU memory, and that the model answers a question:

```bash
python3 -m venv .venv
.venv/bin/pip install --require-hashes -r tests/requirements.txt
.venv/bin/pytest tests/e2e --backend vulkan
```

Name the backend you set. On a Strix Halo, `auto` picks `vulkan`. CI runs them on the CPU, so a pull request that changes the chart should also state which GPU backends and hardware they passed on.

Dependabot updates the actions, the test requirements, and Lemonade's image. Bump these pins by hand:

- the kubeconform, helm-unittest, and actionlint images, and Lemonade's comment check, in [check.yml](workflows/check.yml);
- the Helm, ruff, and Python versions, in check.yml and the [end-to-end action](actions/e2e-cpu/action.yml);
- K3s, in the README and check.yml; AMD's GPU device plugin, in the README; and Rancher Desktop, in check.yml.

## AI tools

lemon8s follows [Lemonade's AI content policy](https://github.com/lemonade-sdk/lemonade/blob/main/docs/dev/ai-content-policy.md). Write your issues, discussions, pull request descriptions, comments, and Discord messages yourself (or at least edit them yourself). You may use AI tools for code and reviews, but you're responsible for what you submit: understand it, review the full diff yourself, remove unrelated changes, test it locally, and credit the tool with a `Co-Authored-By:` trailer.

## License

What you contribute is licensed under [Apache-2.0](../LICENSE), as section 5 of the license sets out. There's no sign-off or contributor agreement, and new files need no license header.
