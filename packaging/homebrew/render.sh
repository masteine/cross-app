#!/usr/bin/env bash
# Fill a Homebrew template's placeholders and syntax-check the result with Ruby.
#
#   render.sh TEMPLATE OUTPUT KEY=VALUE...
#
# Example (values are the release's, there are no defaults on purpose):
#   render.sh cross-app-cli.rb.in dist/cross-app-cli.rb VERSION=0.1.0 \
#     URL=https://github.com/<owner>/<repo>/archive/refs/tags/v0.1.0.tar.gz \
#     SHA256=<sha256 of that tarball> HOMEPAGE=https://github.com/<owner>/<repo> \
#     HEAD_URL=https://github.com/<owner>/<repo>.git
#
# Every @KEY@ in the template must be given, and no @...@ may remain afterwards.
set -euo pipefail

(($# >= 2)) || {
    sed -n '2,12s/^# \{0,1\}//p' "$0" >&2
    exit 2
}
template="$1"
output="$2"
shift 2
[[ -f "$template" ]] || {
    echo "error: template not found: $template" >&2
    exit 1
}

text="$(cat "$template")"
for pair in "$@"; do
    key="${pair%%=*}"
    value="${pair#*=}"
    [[ "$key" =~ ^[A-Z0-9_]+$ && "$pair" == *=* ]] || {
        echo "error: expected KEY=VALUE, got: $pair" >&2
        exit 1
    }
    [[ "$value" != *'"'* && "$value" != *'\'* ]] || {
        echo "error: $key contains a quote or backslash" >&2
        exit 1
    }
    text="${text//@$key@/$value}"
done

left="$(printf '%s\n' "$text" | grep -o '@[A-Z0-9_]*@' | sort -u | tr '\n' ' ' || true)"
[[ -z "$left" ]] || {
    echo "error: unfilled placeholders: $left" >&2
    exit 1
}

mkdir -p "$(dirname "$output")"
printf '%s\n' "$text" >"$output"
ruby -c "$output" >/dev/null
echo "rendered $output"
