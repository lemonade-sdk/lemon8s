<h1 align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/assets/brand/wordmark.dark.svg">
    <img alt="lemon8s" src="docs/assets/brand/wordmark.light.svg" width="440">
  </picture>
</h1>

<h3 align="center">Refreshingly orchestrated local AI</h3>

Goal: one private AI endpoint for your team, with the easy deployment of Lemonade and the controls and scale of Kubernetes.

> [!WARNING]
> lemon8s is an experimental prototype. Expect breaking changes, and don't rely on lemon8s.

Today lemon8s runs [Lemonade](https://github.com/lemonade-sdk/lemonade)'s server on Kubernetes, on one host. People and their AI agents connect with any OpenAI-compatible app and an API key. It's tested with [K3s](https://k3s.io), a lightweight Kubernetes, on a Strix Halo (Ryzen AI Max+ 395), with Lemonade's Vulkan, ROCm, and CPU backends.

Planned: each conversation tagged with the kinds of data it may hold, so that only people cleared for those tags can open it; more hosts behind the same URL; and NPUs as well as GPUs.

## Try it

The chart should run on any Kubernetes with a default storage class. To use a GPU, it needs the GPU vendor's device plugin, and to keep other pods out, a network plugin that enforces NetworkPolicy. Besides K3s, it's tested on the CPU with kind, k0s, minikube, and RKE2. These steps use K3s on Linux with an AMD GPU. lemon8s uses port 13308, so a Lemonade already running on the host can keep its 13305. The two then share the GPU's memory, so each has less room for models.

1. Install K3s without its bundled ingress and load balancer, which lemon8s doesn't use, then [Helm](https://helm.sh/docs/intro/install/) and AMD's GPU device plugin:

   ```bash
   curl -sfL https://raw.githubusercontent.com/k3s-io/k3s/v1.37.2+k3s1/install.sh |
     INSTALL_K3S_VERSION=v1.37.2+k3s1 INSTALL_K3S_EXEC="--disable=traefik --disable=servicelb" sh -
   (umask 077 && mkdir -p ~/.kube && sudo k3s kubectl config view --raw > ~/.kube/k3s.yaml)
   export KUBECONFIG=~/.kube/k3s.yaml
   helm install amd-gpu amd-gpu --repo https://rocm.github.io/k8s-device-plugin --version 0.22.0 \
     --namespace kube-system --set dp.image.tag=1.31.0.9@sha256:26212c665aab415019cfca7a7f1a5c495a306c1e1e03c88f67dfcfda8e81b6e5
   ```

2. Install lemon8s, letting its pod open the GPU through the host's render group, and wait for it:

   ```bash
   git clone https://github.com/lemonade-sdk/lemon8s && cd lemon8s
   kubectl create namespace lemonade
   kubectl label namespace lemonade pod-security.kubernetes.io/enforce=restricted
   helm install lemonade deploy/chart/lemon8s --namespace lemonade \
     --set "deviceGroups={$(getent group render | cut -d: -f3)}"
   kubectl -n lemonade rollout status deploy/lemonade --timeout=15m
   ```

   If it doesn't become ready, `kubectl -n lemonade describe pod` says why, and `kubectl -n lemonade logs deploy/lemonade` shows Lemonade's log.

3. Load a model, and connect an app:

   ```bash
   kubectl -n lemonade exec deploy/lemonade -- lemonade load Qwen3-4B-Instruct-2507-GGUF
   kubectl -n lemonade port-forward svc/lemonade 13308 >/dev/null &
   export OPENAI_BASE_URL=http://localhost:13308/v1
   export OPENAI_API_KEY=$(kubectl -n lemonade get secret lemonade-api-key -o jsonpath='{.data.api-key}' | base64 -d)
   curl "$OPENAI_BASE_URL/chat/completions" -H "Authorization: Bearer $OPENAI_API_KEY" -H "Content-Type: application/json" \
     -d '{"model": "Qwen3-4B-Instruct-2507-GGUF", "messages": [{"role": "user", "content": "Hello"}]}'
   ```

   The OpenAI SDKs read these variables, and other apps take the same URL, key, and model name in their settings.

The [Run Lemonade on Kubernetes](docs/playbooks/lemonade-on-kubernetes.md) playbook compares this with Lemonade on its own and covers changing settings, reaching it from another computer, issuing new keys, uninstalling, troubleshooting, and common questions.

## Contributing

Questions and ideas are welcome in an issue or on [Lemonade's Discord](https://discord.gg/5xXzkMu8Zk). See [CONTRIBUTING.md](.github/CONTRIBUTING.md), report vulnerabilities as [SECURITY.md](.github/SECURITY.md) describes, and follow the [Code of Conduct](.github/CODE_OF_CONDUCT.md).

## License

lemon8s is an open-source project, licensed under the [Apache License 2.0](LICENSE).
