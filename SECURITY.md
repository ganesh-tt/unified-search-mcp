# Security Policy

## Reporting a Vulnerability

If you discover a security vulnerability, please report it responsibly:

1. **Do NOT open a public GitHub issue.**
2. Email the maintainer directly or use GitHub's private vulnerability reporting feature.
3. Include: description of the vulnerability, steps to reproduce, and potential impact.
4. You will receive a response within 48 hours.

## Supported Versions

| Version | Supported |
|---------|-----------|
| 0.3.x   | Yes       |
| < 0.3   | No        |

## Scope

This project handles API tokens for Slack, Confluence, JIRA, and GitHub. Security concerns include:

- Credential handling and storage
- Input validation (injection prevention)
- Data exposure in metrics/logs
- Subprocess command injection
- Supply chain (dependency) vulnerabilities

## Security Features

- Credentials redacted in Debug output
- HTTPS enforced for API connections
- JQL/CQL injection prevention
- Input validation on all identifier parameters
- Restrictive file permissions (0600) on metrics and cache files
- Query text truncated in metrics logs
