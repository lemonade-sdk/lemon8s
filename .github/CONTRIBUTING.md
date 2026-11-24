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

Every pull request runs [the checks](workflows/check.yml): spelling, Markdown, and links; Python lint and format; the chart's lint, schema, and [unit tests](../deploy/chart/lemon8s/tests); the workflows themselves; Lemonade's [comment check](https://github.com/lemonade-sdk/lemonade/blob/main/tools/check_comment_slop.py) on the lines a pull request changes; and the end-to-end tests below, on the CPU, on Linux in K3s, kind, k0s, minikube, RKE2, and kubeadm, each started on a GitHub runner by [cluster.sh](scripts/cluster.sh), and on Windows in [Rancher Desktop](https://rancherdesktop.io), along with a check that Lemonade's NetworkPolicy keeps other pods out. A pull request can merge once the **Checks** job passes, which needs all of these. On your first pull request, the checks start once a maintainer approves them.

In K3s, the end-to-end tests run after an upgrade: the job installs the base branch's chart, downloads a model, and upgrades to the pull request's chart with the [playbook's](../docs/playbooks/lemonade-on-kubernetes.md#change-a-setting) command. After the tests, it rolls back, upgrades again, uninstalls, and installs again, checking that the model stays on its volume throughout and the keys stay the same until the uninstall. So a chart change must keep what an install of `main`'s chart relies on: the values `main` accepts, the selector labels, the PersistentVolumeClaim's name, and the Secret's name and keys. A change that has to break one of these starts with an issue. To try an upgrade yourself, in a test cluster with the namespace from the README's step 2:

```bash
base=$(mktemp -d)
git archive origin/main deploy/chart/lemon8s | tar -x -C "$base"
helm install lemonade "$base/deploy/chart/lemon8s" --namespace lemonade --set devices=null --set backend=cpu
kubectl -n lemonade rollout status deploy/lemonade --timeout=15m
helm -n lemonade upgrade lemonade deploy/chart/lemon8s --reset-then-reuse-values
```

lemon8s supports Kubernetes 1.35 to 1.37. CI runs 1.35 on kind and 1.37 everywhere else, and checks the chart against 1.35, 1.36, and 1.37, so use only APIs that 1.35 serves.

The end-to-end tests in [`tests/e2e`](../tests/e2e) can be run against a live install, through the port-forward and variables from the README's step 3. The tests unload every model, so use a test install. The tests check that the API key is required and can't change Lemonade's settings, that the model runs with the backend you name, that a model on the GPU takes GPU memory, and that the model answers a question:

```bash
python3 -m venv .venv
.venv/bin/pip install --require-hashes -r tests/requirements.txt
.venv/bin/pytest tests --backend vulkan
```

Install or upgrade with the backend you mean to test, such as `--set backend=vulkan`, and pass the same name to `--backend`. CI runs the end-to-end tests only on the CPU, so a pull request that changes the chart should also state which GPU backends and hardware they passed on. If you have no GPU to test on, say so, and a maintainer will run them.

Dependabot updates the actions, the test requirements, and Lemonade's image. Other pinned versions, such as those at the top of [check.yml](workflows/check.yml), the images, installers, packages, and charts in its jobs and in [cluster.sh](scripts/cluster.sh), Python in the [end-to-end action](actions/e2e-cpu/action.yml), the README's K3s and device plugin, and the oldest supported Kubernetes, 1.35, in kind and in the chart's schema check, are updated by hand, along with the checksum or digest beside each.

## AI tools

lemon8s follows [Lemonade's AI content policy](https://github.com/lemonade-sdk/lemonade/blob/main/docs/dev/ai-content-policy.md). Write your issues, discussions, pull request descriptions, comments, and Discord messages yourself (or at least edit them yourself). You may use AI tools for code and reviews, but you're responsible for what you submit: understand it, review the full diff yourself, remove unrelated changes, test it locally, and credit the tool with a `Co-Authored-By:` trailer.

## License

What you contribute is licensed under [Apache-2.0](../LICENSE), as section 5 of the license sets out. There's no sign-off or contributor agreement, and new files need no license header.
