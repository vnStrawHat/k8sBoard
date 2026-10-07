# 0042 addendum: New Secret and Replace certificate (UX round 3, N11 and N16)

The "Secrets: deferred" non-goal of the README is lifted. Secret is the sixth creatable kind. Modules: `secret_forms.rs` (pure builders), `secret_form.rs` (the form dialog), `registry_secret.rs` and `certificate.rs` (cluster: the dockerconfigjson and the TLS pair check).

## Entry points

- Secrets header `New ˅` menu: `Opaque…` (the YAML editor with a `stringData` template), `docker-registry…`, `TLS…`. Gate: `create secrets`, like every `New`.
- The WHY of a failed pull: a pull secret that is known to be missing reads `name (missing) Create it →`; while the namespace's Secrets are not loaded, `Create pull secret →` follows the links. Both open `New docker-registry Secret <name> in <ns>` with the name filled in. The create right is asked on the spot (the lazy review only runs for the shown screen), and the form opens when the answer is in.
- A TLS Secret row (menu, drawer menu, palette): `Replace certificate…` (action `ReplaceCertificate`, gate `patch secrets`, refused for an immutable Secret or a Helm record like Edit values).

## The forms

- docker-registry: name, namespace, server (default `https://index.docker.io/v1/`), user name, password (masked input), email (optional). The code builds `.dockerconfigjson` as `kubectl create secret docker-registry` does.
- TLS (new and replace): a certificate field and a private key field. The key is hidden by default (`•••• N chars`, Paste and Show; both are disabled in screenshot builds like every secret value). Under the fields: Subject, Not after with the days left, and the key check. A key of another certificate refuses Review…; a key that cannot be compared (encrypted, Ed25519, an EC key without its public point) is allowed with a warning.
- The key check reads only the PEM and DER structure: an RSA private key holds the modulus and exponent, an EC key usually holds the point. No crate was added (`x509-cert` and `pem` were already there).
- Replace reads the Secret fresh (`values_base`) and sends one `SetDataValues` of `tls.crt` and `tls.key` guarded by its `resourceVersion`; the confirm lists `not after: <old> → <new>`.

## Write path

`CreateObject` accepts a Secret. `ObjectDraft` folds `stringData` into base64 `data` (as the server does), so the dry-run answer compares path for path; the confirm and the audit line list the type and the key names (`data[tls.key]`), never a value. The form stays open under the confirm dialog. A server refusal of the dry-run (a 422, a 403, `AlreadyExists`, a webhook that cannot dry-run) closes the confirm dialog and shows the server's words under the fields; so does a commit that fails without a Retry. The fields are kept, and the next Review… clears the message and checks again. A transient failure (a conflict, a 429) stays in the confirm dialog with its Retry. A commit that went through closes the confirm dialog and then the form.
