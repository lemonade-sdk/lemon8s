{{- define "lemon8s.selectorLabels" -}}
app.kubernetes.io/name: lemon8s
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/component: lemonade
{{- end }}

{{- define "lemon8s.labels" -}}
{{ include "lemon8s.selectorLabels" . }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
helm.sh/chart: {{ .Chart.Name }}-{{ .Chart.Version }}
{{- end }}
