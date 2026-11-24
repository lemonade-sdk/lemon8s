#!/usr/bin/env bash
set -uo pipefail

flavor=${1:?usage: cluster-state.sh k3s|kind|k0s|minikube|rke2|kubeadm}

kubectl get nodes,pods,pvc,networkpolicies,events -A
kubectl -n lemonade describe pod,pvc
kubectl -n lemonade logs deploy/lemonade --tail=200
cat "$RUNNER_TEMP/port-forward.log"
case "$flavor" in
  k3s) sudo journalctl -u k3s --no-pager | tail -n 200 ;;
  kind) kind export logs "$RUNNER_TEMP/kind-logs" --name lemon8s && tail -n 200 "$RUNNER_TEMP"/kind-logs/*/kubelet.log ;;
  k0s) docker logs --tail 200 k0s ;;
  minikube) minikube logs --length 200 ;;
  rke2) sudo journalctl -u rke2-server --no-pager | tail -n 200 ;;
  kubeadm) sudo journalctl -u kubelet --no-pager | tail -n 200; kubectl -n kube-system logs ds/cilium --tail=200 ;;
esac
exit 0
