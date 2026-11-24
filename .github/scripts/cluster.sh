#!/usr/bin/env bash
# Start a one-node cluster on a GitHub runner, with a default storage class and a network plugin
# that enforces NetworkPolicy.
set -euo pipefail

flavor=${1:?usage: cluster.sh k3s|kind|k0s|minikube|rke2|kubeadm}
bin=$RUNNER_TEMP/bin
kubeconfig=$HOME/.kube/$flavor.yaml
mkdir -p "$bin" "$HOME/.kube"
export PATH="$bin:$PATH" KUBECONFIG=$kubeconfig
echo "$bin" >> "$GITHUB_PATH"
echo "KUBECONFIG=$kubeconfig" >> "$GITHUB_ENV"

fetch() {
  curl -fsSL -o "$3" "$1"
  echo "$2  $3" | sha256sum -c -
}

case "$flavor" in
  k3s)
    fetch "https://raw.githubusercontent.com/k3s-io/k3s/$K3S_VERSION/install.sh" "$K3S_INSTALL_SHA256" "$RUNNER_TEMP/k3s-install.sh"
    INSTALL_K3S_VERSION="$K3S_VERSION" INSTALL_K3S_SYMLINK=force \
      INSTALL_K3S_EXEC="--disable=traefik --disable=servicelb" sh "$RUNNER_TEMP/k3s-install.sh"
    (umask 077 && sudo k3s kubectl config view --raw | tee "$kubeconfig" >/dev/null)
    ;;
  kind)
    fetch https://github.com/kubernetes-sigs/kind/releases/download/v0.33.0/kind-linux-amd64 \
      aee6151561422756b764a4ae28e7f44cda5af5a9eead3cc9985112b1de8d8e0d "$bin/kind"
    chmod +x "$bin/kind"
    kind create cluster --name lemon8s --wait 5m --retain \
      --image kindest/node:v1.37.0@sha256:a1ed56cfb0e7b93589bdf97c8cd566405a265939e3620fc4f5de89adff580ae5
    ;;
  k0s)
    docker run -d --name k0s --hostname k0s -v /var/lib/k0s -v /var/log/pods --tmpfs /run --privileged \
      -p 127.0.0.1::6443 \
      docker.io/k0sproject/k0s:v1.37.2-k0s.1@sha256:b315f63e2e23430e68c9fcaf14fb13a27413dd92b973c24a4b5272968923a644 \
      k0s controller --enable-worker --no-taints
    api=$(docker port k0s 6443/tcp | head -n 1)
    # A docker exec while k0s's entrypoint sets up cgroups makes the entrypoint fail, so wait from outside.
    timeout 300 bash -c "until curl -ks -o /dev/null https://$api/readyz; do sleep 2; done"
    (umask 077 && docker exec k0s k0s kubeconfig admin > "$kubeconfig")
    sed -i "s#server: https://.*#server: https://$api#" "$kubeconfig"
    ;;
  minikube)
    fetch https://github.com/kubernetes/minikube/releases/download/v1.39.0/minikube-linux-amd64 \
      b738496da01be06bbaf80c688f57ce25acd3849fbb518155f3a88e03ef555aa4 "$bin/minikube"
    chmod +x "$bin/minikube"
    minikube start --driver=docker --cni=calico --kubernetes-version=v1.37.2 --wait=all
    ;;
  rke2)
    fetch "https://raw.githubusercontent.com/rancher/rke2/v1.37.2+rke2r1/install.sh" \
      42983c86d1da64a92061d83afb57630cedd69241989f1b0673f3db6c3d92ee6b "$RUNNER_TEMP/rke2-install.sh"
    sudo INSTALL_RKE2_VERSION=v1.37.2+rke2r1 sh "$RUNNER_TEMP/rke2-install.sh"
    sudo systemctl enable --now rke2-server.service
    (umask 077 && sudo cat /etc/rancher/rke2/rke2.yaml | tee "$kubeconfig" >/dev/null)
    ;;
  kubeadm)
    fetch https://pkgs.k8s.io/core:/stable:/v1.37/deb/Release.key \
      7627818cf7bae52f9008c93e8b1f961f53dea11d40891778de216fb1b43be54d "$RUNNER_TEMP/kubernetes.asc"
    sudo install -m 644 "$RUNNER_TEMP/kubernetes.asc" /etc/apt/keyrings/
    echo "deb [signed-by=/etc/apt/keyrings/kubernetes.asc] https://pkgs.k8s.io/core:/stable:/v1.37/deb/ /" |
      sudo tee /etc/apt/sources.list.d/kubernetes.list >/dev/null
    sudo apt-get update
    sudo apt-get install -y kubelet=1.37.2-1.1 kubeadm=1.37.2-1.1
    # The runner's containerd comes from Docker's package, whose config disables the CRI plugin.
    containerd config default | sed 's/SystemdCgroup = false/SystemdCgroup = true/' | sudo tee /etc/containerd/config.toml >/dev/null
    sudo systemctl restart containerd
    sudo swapoff -a
    sudo sysctl -w net.ipv4.ip_forward=1
    sudo kubeadm init --kubernetes-version=v1.37.2 --pod-network-cidr=10.244.0.0/16
    (umask 077 && sudo cat /etc/kubernetes/admin.conf | tee "$kubeconfig" >/dev/null)
    kubectl taint nodes --all node-role.kubernetes.io/control-plane-
    fetch https://helm.cilium.io/cilium-1.20.5.tgz \
      0c7592eb75179ecf46dd3ec22145916f0164c4401214ea15afb5b230b1d11a21 "$RUNNER_TEMP/cilium.tgz"
    helm install cilium "$RUNNER_TEMP/cilium.tgz" --namespace kube-system \
      --set ipam.mode=kubernetes --set operator.replicas=1 --wait --timeout 5m
    ;;
  *)
    echo "unknown flavor: $flavor" >&2
    exit 2
    ;;
esac

timeout 300 bash -c 'until kubectl get nodes | grep -q " Ready"; do sleep 2; done' || { kubectl get nodes -o wide; exit 1; }

# k0s, RKE2, and kubeadm ship no default storage class; Rancher's local-path provisioner is the one K3s bundles.
if [ "$flavor" = k0s ] || [ "$flavor" = rke2 ] || [ "$flavor" = kubeadm ]; then
  manifest=$RUNNER_TEMP/local-path-storage.yaml
  fetch https://raw.githubusercontent.com/rancher/local-path-provisioner/v0.0.37/deploy/local-path-storage.yaml \
    9781b39c24f3f651bd6d6e41b561e04e4904bbdb6d4f8c7a6009df3a702dcd65 "$manifest"
  sed -i -e 's#image: docker.io/rancher/local-path-provisioner:v0.0.37$#&@sha256:e757967a5ec338f6a9b371c5a9688bedaa8c3578ea3dd4db329ea0084be0a86f#' \
    -e 's#image: docker.io/library/busybox$#&:1.37@sha256:a7dbc319b1438115a207e11930dc4bb3e643dcb3ec0bdd694bb36cd8fad289ec#' "$manifest"
  kubectl apply -f "$manifest"
  kubectl patch storageclass local-path -p '{"metadata":{"annotations":{"storageclass.kubernetes.io/is-default-class":"true"}}}'
  kubectl -n local-path-storage rollout status deploy/local-path-provisioner --timeout=5m
fi

timeout 120 bash -c "until kubectl get storageclass -o jsonpath='{.items[?(@.metadata.annotations.storageclass\.kubernetes\.io/is-default-class==\"true\")].metadata.name}' | grep -q .; do sleep 2; done" ||
  { echo "no default storage class" >&2; kubectl get storageclass; exit 1; }
