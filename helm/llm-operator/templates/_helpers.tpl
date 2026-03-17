{{/*
Expand the name of the chart.
*/}}
{{- define "llm-operator.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Create a default fully qualified app name.
Truncated to 63 chars because Kubernetes names must fit in DNS labels.
*/}}
{{- define "llm-operator.fullname" -}}
{{- if .Values.fullnameOverride }}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- $name := default .Chart.Name .Values.nameOverride }}
{{- if contains $name .Release.Name }}
{{- .Release.Name | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- printf "%s-%s" .Release.Name $name | trunc 63 | trimSuffix "-" }}
{{- end }}
{{- end }}
{{- end }}

{{/*
Chart label: name-version, used in helm.sh/chart label.
*/}}
{{- define "llm-operator.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Common labels — applied to every resource managed by this chart.
*/}}
{{- define "llm-operator.labels" -}}
helm.sh/chart: {{ include "llm-operator.chart" . }}
{{ include "llm-operator.selectorLabels" . }}
{{- if .Chart.AppVersion }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
{{- end }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end }}

{{/*
Selector labels — used in Deployment.spec.selector and Service.spec.selector.
Must be stable across upgrades (never include chart version here).
*/}}
{{- define "llm-operator.selectorLabels" -}}
app.kubernetes.io/name: {{ include "llm-operator.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{/*
Resolve the service account name.
*/}}
{{- define "llm-operator.serviceAccountName" -}}
{{- if .Values.serviceAccount.create }}
{{- default (include "llm-operator.fullname" .) .Values.serviceAccount.name }}
{{- else }}
{{- default "default" .Values.serviceAccount.name }}
{{- end }}
{{- end }}
