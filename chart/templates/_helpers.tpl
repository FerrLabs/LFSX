{{- define "lfsx.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "lfsx.fullname" -}}
{{- if .Values.fullnameOverride -}}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- printf "%s-%s" .Release.Name (include "lfsx.name" .) | trunc 63 | trimSuffix "-" -}}
{{- end -}}
{{- end -}}

{{- define "lfsx.labels" -}}
helm.sh/chart: {{ printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" }}
app.kubernetes.io/name: {{ include "lfsx.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end -}}

{{- define "lfsx.selectorLabels" -}}
app.kubernetes.io/name: {{ include "lfsx.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end -}}

{{- define "lfsx.serviceAccountName" -}}
{{- if .Values.serviceAccount.create -}}
{{- default (include "lfsx.fullname" .) .Values.serviceAccount.name -}}
{{- else -}}
{{- default "default" .Values.serviceAccount.name -}}
{{- end -}}
{{- end -}}

{{- define "lfsx.claimName" -}}
{{- default (include "lfsx.fullname" .) .Values.persistence.existingClaim -}}
{{- end -}}

{{- define "lfsx.publicUrl" -}}
{{- if .Values.config.publicUrl -}}
{{- .Values.config.publicUrl | trimSuffix "/" -}}
{{- else if and .Values.ingress.enabled .Values.ingress.host -}}
{{- if .Values.ingress.tls.enabled -}}
{{- printf "https://%s" .Values.ingress.host -}}
{{- else -}}
{{- printf "http://%s" .Values.ingress.host -}}
{{- end -}}
{{- else -}}
{{- fail "set config.publicUrl, or enable the ingress and set ingress.host: it is echoed in the batch response and every transfer reconnects to it, so a wrong value makes negotiation succeed and all transfers fail" -}}
{{- end -}}
{{- end -}}

{{- define "lfsx.ingressAnnotations" -}}
{{- $annotations := .Values.ingress.annotations | default dict -}}
{{- if eq .Values.ingress.className "nginx" -}}
{{- $annotations = merge (dict "nginx.ingress.kubernetes.io/proxy-body-size" "0" "nginx.ingress.kubernetes.io/proxy-request-buffering" "off" "nginx.ingress.kubernetes.io/proxy-read-timeout" "3600" "nginx.ingress.kubernetes.io/proxy-send-timeout" "3600") $annotations -}}
{{- end -}}
{{- toYaml $annotations -}}
{{- end -}}
