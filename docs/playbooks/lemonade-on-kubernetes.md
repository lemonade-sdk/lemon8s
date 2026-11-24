# Run Lemonade on Kubernetes

This playbook runs Lemonade's server on Kubernetes, on one host. Install it with the README's [Try it](../../README.md#try-it) steps. This playbook covers how it differs from Lemonade on its own, running it on Windows, day-to-day changes, what to do when something goes wrong, and common questions.

## How it compares with Lemonade on its own

Both run the same Lemonade server. lemon8s runs it in a locked-down pod, keeps its files on a Kubernetes volume, and requires an API key.

Lemonade on its own, installed from its Linux package:

```mermaid
flowchart TB
  apps["Apps on the host"] -->|"localhost:13305, no key unless you set one"| lemond["lemond, a systemd service run as the lemonade user"]
  lemond --- files[("Models, backends, and config in /var/lib/lemonade and /var/cache/lemonade")]
  lemond --- gpu["GPU"]
```

lemon8s today, on K3s:

```mermaid
flowchart TB
  apps["Apps on the host"] -->|"kubectl port-forward svc/lemonade 13308, API key required"| pod["Pod with Lemonade's own image: non-root, read-only root filesystem, no capabilities"]
  policy["NetworkPolicy: nothing but the host reaches the pod"] -.- pod
  volume[("Volume lemonade-models: models, backends, and config")] --- pod
  secret["Secret: an API key for apps and a separate admin key"] --- pod
  plugin["AMD's GPU device plugin"] -->|"amd.com/gpu"| pod
  pod --- gpu["GPU"]
```

## Run it on Windows

On Windows, lemon8s runs on the CPU in [Rancher Desktop](https://rancherdesktop.io), which runs K3s in WSL2 and comes with kubectl and Helm:

1. Install [Rancher Desktop](https://docs.rancherdesktop.io/getting-started/installation/), which needs Windows 11 or Windows Server 2025, with WSL installed first. When it first starts, keep Kubernetes enabled and choose containerd as the container engine.
2. In Git Bash, which comes with Git for Windows, follow the README's steps 2 and 3, but install lemon8s with no devices and Lemonade's CPU backend:

   ```bash
   helm install lemonade deploy/chart/lemon8s --namespace lemonade --set devices=null --set backend=cpu
   ```

The GPU, which WSL2 offers only as `/dev/dxg`, and the NPU don't work there yet.

## Change a setting

[values.yaml](../../deploy/chart/lemon8s/values.yaml) lists the settings. Change one with an upgrade, which keeps your other settings and takes the chart's new defaults, then start the port-forward again, because it follows one pod and the pod restarts:

```bash
helm -n lemonade upgrade lemonade deploy/chart/lemon8s --reset-then-reuse-values --set backend=rocm
```

Other settings work the same way:

- **Run on the CPU only:** `--set devices=null --set backend=cpu`. To give the pod the GPU again: `--set "devices.amd\.com/gpu=1" --set backend=auto`.
- **Another Service port:** `--set service.port=13310`, then forward that port instead of 13308. To change only the port on the host, forward another local port instead, as Troubleshooting shows.
- **Any of Lemonade's settings,** by its name in Lemonade's `config.json`, such as `--set settings.ctx_size=8192`; for the llama.cpp backend, use `backend`. A setting changed through Lemonade itself takes precedence over the chart's.
- **CPU and memory for the pod,** on a host it shares: `--set resources.requests.cpu=2 --set resources.requests.memory=8Gi`.
- **Which node it runs on,** in a cluster with more than one: `nodeSelector`, `tolerations`, and `affinity`. On K3s's default storage, set them at install, because the volume stays on the node where the pod first ran.
- **Keys from a Secret of your own,** with two different, non-empty keys, `api-key` and `admin-api-key`: `--set apiKey.existingSecret=<name>`.

The volume's size and storage class are set at install, and an upgrade can't change them.

## Reach it from another computer

Forward the port over SSH, then use the same URL, key, and model name there:

```bash
ssh -N -L 13308:localhost:13308 <you>@<host>
```

## Issue new keys

Apps use the `api-key` in the `lemonade-api-key` Secret. The `admin-api-key` beside it can also change Lemonade's settings, so keep it to yourself; `kubectl -n lemonade exec deploy/lemonade -- lemonade ...` uses it from inside the pod. To replace both keys:

```bash
kubectl -n lemonade delete secret lemonade-api-key
helm -n lemonade upgrade lemonade deploy/chart/lemon8s --reset-then-reuse-values
kubectl -n lemonade rollout restart deploy/lemonade
```

The old keys stop working once the pod restarts. With a Secret of your own, replace its keys, then run only the last command.

## Uninstall

`helm -n lemonade uninstall lemonade` removes lemon8s and its keys but keeps the models' volume, so a reinstall doesn't download them again. `kubectl -n lemonade delete pvc lemonade-models` removes the volume. K3s has its own [uninstall script](https://docs.k3s.io/installation/uninstall).

## Troubleshooting

**The pod stays Pending.** `kubectl -n lemonade describe pod` shows why. `Insufficient amd.com/gpu` means no device plugin offers the GPU: check that it runs, with `kubectl -n kube-system get pods`, and that the node offers it, with `kubectl get node -o jsonpath='{.items[0].status.allocatable}'`.

**`helm install` stops with "Set deviceGroups".** The pod needs the host groups that may open the GPU's device files. Pass `--set "deviceGroups={$(getent group render | cut -d: -f3)}"`.

**`lemonade load` fails with "Couldn't resolve host name".** The pod can't resolve names, even if the host can. Check with `kubectl -n lemonade exec deploy/lemonade -- getent hosts huggingface.co`. On networks where some of the host's DNS servers don't answer, point K3s at one that does (`resolvectl status` shows which one the host uses):

```bash
echo "nameserver <a DNS server that answers>" | sudo tee /etc/rancher/k3s/resolv.conf
echo "resolv-conf: /etc/rancher/k3s/resolv.conf" | sudo tee -a /etc/rancher/k3s/config.yaml
sudo systemctl restart k3s
kubectl -n kube-system rollout restart deploy/coredns
```

**The port-forward fails.** Another program on the host holds port 13308. Forward another local port, such as `kubectl -n lemonade port-forward svc/lemonade 13309:13308`, and use it in the URL.

**Apps get "connection refused" after an upgrade or restart.** The port-forward ended with the old pod. Start it again.

**A small model takes most of the GPU's memory.** Lemonade sizes the context automatically. On a Strix Halo, Qwen3-4B-Instruct-2507 got a 262,144-token context and took about 39 GiB; with `--ctx-size 8192` it took under 4 GiB:

```bash
kubectl -n lemonade exec deploy/lemonade -- lemonade load Qwen3-4B-Instruct-2507-GGUF --ctx-size 8192
```

To make that the default for every model, set `settings.ctx_size=8192` in an upgrade.

**A model runs on the CPU, though the host has a GPU.** With `backend=auto`, Lemonade picks the best llama.cpp build already on the volume, and downloads one only when there's none. So once only the CPU build is there, `auto` keeps using it. Set the backend you want, and give the pod the GPU again if you ran it on the CPU only: `--set backend=vulkan --set "devices.amd\.com/gpu=1"`, in an upgrade.

**The host's disk fills up.** On K3s, the volume lives on the host's disk, under `/var/lib/rancher/k3s/storage` by default, and its size isn't enforced. Delete models you don't need with `kubectl -n lemonade exec deploy/lemonade -- lemonade delete <model>`, or install K3s with `--default-local-storage-path` pointing at a larger disk.

## Questions

**How useful is it today?** It gives apps on one host, and on other computers through SSH, one Lemonade server with an API key, managed by Kubernetes. For one person, Lemonade on its own does the same with fewer steps. lemon8s becomes more useful after more features land: data tags, more hosts behind one URL, and access for each person.

**How safe is it?** It keeps three promises: only its host can reach it, every request but Lemonade's liveness check needs the API key, and that key can't change Lemonade's settings. The pod runs without root, privileges, or a writable root filesystem. There's no TLS yet, and everyone shares one key.

**Does it run on Windows?** Yes, on the CPU, in Rancher Desktop, as [Run it on Windows](#run-it-on-windows) shows. Kubernetes needs Linux for its control plane, so Rancher Desktop runs K3s in WSL2. Using the GPU there is planned.

**Does it work with other GPUs, or with NPUs?** The chart requests any device a device plugin offers, through `devices`, but only AMD's GPU device plugin on a Strix Halo has been tested. NPUs are planned.

**Why Kubernetes instead of Lemonade on its own?** For one person on one host, Lemonade on its own is simpler. Kubernetes is the base for what's planned: several hosts behind one URL, access for each person, and standard tools, such as Helm, role-based access control, and NetworkPolicy, to run it.
