# Security Policy

## Supported versions

Only the latest release gets fixes. Pre-1.0, that means the newest tag; older
tags are not patched.

| Version | Supported          |
| ------- | ------------------ |
| 0.1.x   | :white_check_mark: |
| < 0.1   | :x:                |

## Reporting a vulnerability

Please don't open a public issue for security problems.

Report it privately through GitHub: [**Report a
vulnerability**](https://github.com/GrowlyX/neolink/security/advisories/new)
(the Security tab, then "Report a vulnerability").

Include what you can:

- Neolink version, macOS build, and how you installed it
- the dock or adapter model, and whether the vendor agent was running
- steps to reproduce, or a proof of concept
- what an attacker gains

### What to expect

- **Acknowledgement** within 3 business days.
- **Assessment** within 10 business days: we confirm the issue, or explain why
  we don't consider it a vulnerability.
- **Fix and disclosure**: we ship a patched release, publish a GitHub security
  advisory (with a CVE when warranted), and credit you unless you'd rather we
  didn't. We aim to fix within 90 days.

## Scope

In scope:

- The `neolink` app and `neolinkctl`, including the USB transport
- The vendor package fetcher: the pinned URL, SHA-256 check, cache layout under
  `~/Library/Application Support/Neolink`, and the `installer` hand-off
- The XPC client that talks to Synaptics' service (parsing of its replies, and
  anything a hostile or impersonating service could do to us)
- The `CGVirtualDisplay` engine, and saved layout profiles
- The bundling script and the signature it produces

Out of scope:

- Vulnerabilities in macOS, CoreGraphics' private APIs, or Synaptics'
  `DisplayLink Manager` itself — report those upstream, though tell us if we
  make them worse
- Attacks that already need the same user account, or root
- The ad-hoc signature not satisfying Gatekeeper on a machine that never built
  the bundle (that's expected until we notarise)

Thanks for helping keep Neolink and its users safe.
