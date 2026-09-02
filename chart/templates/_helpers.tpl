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
{{- end -}}
{{- end -}}

{{- define "lfsx.ingressAnnotations" -}}
{{- $annotations := .Values.ingress.annotations | default dict -}}
{{- if eq .Values.ingress.className "nginx" -}}
{{- $annotations = merge (dict "nginx.ingress.kubernetes.io/proxy-body-size" "0" "nginx.ingress.kubernetes.io/proxy-request-buffering" "off" "nginx.ingress.kubernetes.io/proxy-read-timeout" "3600" "nginx.ingress.kubernetes.io/proxy-send-timeout" "3600") $annotations -}}
{{- end -}}
{{- toYaml $annotations -}}
{{- end -}}

{{- define "lfsx.forgeApiUrl" -}}
{{- if eq .Values.auth.mode "gitlab" -}}
{{- .Values.auth.gitlabApiUrl -}}
{{- else if eq .Values.auth.mode "gitea" -}}
{{- .Values.auth.giteaApiUrl -}}
{{- else -}}
{{- .Values.auth.githubApiUrl -}}
{{- end -}}
{{- end -}}

{{- define "lfsx.validate" -}}
{{- if eq .Values.storage.type "s3" -}}
  {{- if not .Values.storage.s3.endpoint -}}
    {{- fail "storage.type=s3 needs storage.s3.endpoint" -}}
  {{- end -}}
  {{- if not .Values.storage.s3.bucket -}}
    {{- fail "storage.type=s3 needs storage.s3.bucket" -}}
  {{- end -}}
  {{- if not .Values.storage.s3.existingSecret -}}
    {{- fail "storage.type=s3 needs storage.s3.existingSecret: the chart never takes the keys as values, because a Helm value ends up in the release secret and in whatever CI printed the command" -}}
  {{- end -}}
{{- else if ne .Values.storage.type "local" -}}
  {{- fail (printf "storage.type must be local or s3, got %q" .Values.storage.type) -}}
{{- end -}}
{{- if eq .Values.auth.mode "gitea" -}}
  {{- if not .Values.auth.giteaApiUrl -}}
    {{- fail "auth.mode=gitea needs auth.giteaApiUrl: Gitea and Forgejo have no default API host, point it at https://git.example.com/api/v1" -}}
  {{- end -}}
{{- end -}}
{{- if gt (int .Values.replicaCount) 1 -}}
  {{- if ne .Values.storage.type "s3" -}}
    {{- fail "replicaCount above 1 needs storage.type=s3: on a volume an upload is staged and renamed, which is atomic on one filesystem and undefined across two, and the locks two pods must agree on live in that same directory" -}}
  {{- end -}}
  {{- if .Values.persistence.enabled -}}
    {{- fail "replicaCount above 1 needs persistence.enabled=false: the claim is ReadWriteOnce, so a second pod cannot mount it, and each replica stages its own uploads anyway" -}}
  {{- end -}}
{{- end -}}
{{- end -}}
