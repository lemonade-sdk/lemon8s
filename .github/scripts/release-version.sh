#!/usr/bin/env bash
# Set the chart's version and the docs' install commands to a new version, if one is given, then
# check that they agree and that the chart is marked alpha. Prints the version.
set -euo pipefail

chart=deploy/chart/lemon8s/Chart.yaml
docs=(README.md docs/playbooks/lemonade-on-kubernetes.md)
alpha='^0\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)-alpha$'
version=$(sed -n 's/^version: //p' "$chart")

if (($#)); then
  if [[ ! $1 =~ $alpha ]]; then
    echo "::error::$1 isn't 0.MINOR.PATCH-alpha" >&2
    exit 1
  fi
  if ! printf '%s\n' "${version%-alpha}" "${1%-alpha}" | sort -V -C -u; then
    echo "::error::$1 isn't newer than $version" >&2
    exit 1
  fi
  sed -i "s/^version: .*/version: $1/" "$chart"
  sed -i -E "s|(charts/lemon8s --version )[^ ]+|\1$1|" "${docs[@]}"
  version=$1
fi

if [[ ! $version =~ $alpha ]]; then
  echo "::error::$chart's version, $version, isn't 0.MINOR.PATCH-alpha" >&2
  exit 1
fi
if ! grep -qx 'description: "Alpha: .*"' "$chart" || ! grep -qx '  lemon8s.io/maturity: alpha' "$chart"; then
  echo "::error::$chart must keep its description starting \"Alpha: \" and its lemon8s.io/maturity: alpha annotation" >&2
  exit 1
fi
stale=$(grep -n 'charts/lemon8s' "${docs[@]}" | grep -v "charts/lemon8s --version $version\( \|$\)" || true)
if [[ -n $stale || $(grep -c "charts/lemon8s --version $version" README.md) == 0 ]]; then
  echo "::error::Every install command in ${docs[*]} must name the chart with --version $version: $stale" >&2
  exit 1
fi
echo "$version"
