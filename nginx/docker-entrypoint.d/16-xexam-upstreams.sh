#!/bin/sh
# Renders /tmp/xexam-upstreams.conf (included in http {} by nginx.conf) from the environment:
#   XEXAM_BACKEND_UPSTREAM  where the main backend is.   http(s)://host[:port], default http://backend:8765
#   XEXAM_AI_UPSTREAM       where the AI service is.     http(s)://host[:port], default http://ai-service:8766
#   XEXAM_NGINX_RESOLVER    the DNS servers Nginx asks at run time. IPv4 or IPv6 (plain or in [brackets]) with an optional
#                           :port (a port on an IPv6 address needs the brackets), separated by spaces or commas.
#                           Default 127.0.0.11 (Docker's built-in DNS); on another machine's setup put your own DNS here.
# Unset, empty, only spaces or only quote characters = the default. Quote characters around a value are stripped. One
# trailing / of an upstream is removed. There is no path, query, user name or any other character: the value is written into
# the Nginx configuration, so anything outside the plain pattern is refused (exit 1, nothing is written).
# With an https upstream the certificate is verified against the system CA bundle (proxy_ssl_verify on).
# Plain POSIX sh plus busybox sed / grep / tr (the base image is Alpine).
set -eu
set -f   # never glob-expand a value

OUT="${XEXAM_UPSTREAMS_OUT:-/tmp/xexam-upstreams.conf}"   # the override exists for local testing of this script
CA_FILE="/etc/ssl/certs/ca-certificates.crt"
DEFAULT_BACKEND="http://backend:8765"
DEFAULT_AI="http://ai-service:8766"
DEFAULT_RESOLVER="127.0.0.11"

fail() {
    rm -f "$OUT"
    echo "xexam-upstreams: $1" >&2
    exit 1
}

# The value cut to 200 printable characters, for the error message only.
shown() {
    printf '%s' "$1" | tr -cd ' -~' | cut -c1-200
}

# Quote characters (ASCII, curly and full-width) become spaces, then the ends are trimmed.
QRE="\"|'|“|”|‘|’|＂"
clean() {
    printf '%s' "$1" | sed -E "s/$QRE/ /g" | tr '\t\r\n' '   ' | sed -E 's/^ +//; s/ +$//'
}

# upstream_value NAME DEFAULT: prints the validated upstream URL (no trailing slash).
upstream_value() {
    name="$1"
    v=$(clean "$(eval "printf '%s' \"\${$name-}\"")")
    [ -n "$v" ] || v="$2"
    v="${v%/}"
    bad="$name must be http(s)://host[:port] with no path, for example http://10.0.0.2:8765 (got '$(shown "$v")')"
    printf '%s' "$v" | grep -Eq '^https?://(\[[0-9A-Fa-f:.]+\]|[A-Za-z0-9]([A-Za-z0-9.-]{0,251}[A-Za-z0-9])?)(:[0-9]{1,5})?$' || fail "$bad"
    # the port, when given, is 1..65535 (leading zeros removed first: a shell would read 0080 as octal)
    case "$v" in
        *\]:*) port="${v##*\]:}" ;;
        \[*\]*|http://\[*|https://\[*) port="" ;;
        *) rest="${v#*://}"; case "$rest" in *:*) port="${rest##*:}" ;; *) port="" ;; esac ;;
    esac
    if [ -n "$port" ]; then
        port=$(printf '%s' "$port" | sed -E 's/^0+//')
        [ -n "$port" ] && [ "${#port}" -le 5 ] && [ "$port" -ge 1 ] && [ "$port" -le 65535 ] || fail "$bad"
    fi
    printf '%s' "$v"
}

BACKEND=$(upstream_value XEXAM_BACKEND_UPSTREAM "$DEFAULT_BACKEND")
AI=$(upstream_value XEXAM_AI_UPSTREAM "$DEFAULT_AI")

# —— the resolver ——
RES_BAD="XEXAM_NGINX_RESOLVER must be one or more DNS server addresses (IPv4 or IPv6, optional :port), for example 10.0.0.53 or 127.0.0.11 (got '__GOT__')"
RAW=$(clean "${XEXAM_NGINX_RESOLVER-}")
[ -n "$RAW" ] || RAW="$DEFAULT_RESOLVER"
res_fail() {
    fail "$(printf '%s' "$RES_BAD" | sed "s|__GOT__|$(shown "$RAW" | sed 's/[\|&]/_/g')|")"
}
# One IPv4 address into $ADDR (valid octets), returns 1 when it is not.
ipv4_ok() {
    printf '%s' "$1" | grep -Eq '^(0|[1-9][0-9]{0,2})(\.(0|[1-9][0-9]{0,2})){3}$' || return 1
    oldifs="$IFS"; IFS=.
    set -- $1
    IFS="$oldifs"
    for o in "$@"; do
        [ "$o" -le 255 ] || return 1
    done
    return 0
}
# One IPv6 address (no brackets, no port); returns 1 when it is not standard.
ipv6_ok() {
    h='[0-9A-Fa-f]{1,4}'
    case "$1" in
        *::*)
            left="${1%%::*}"
            right="${1#*::}"
            case "$right" in *::*) return 1 ;; esac
            for side in "$left" "$right"; do
                printf '%s\n' "$side" | grep -Eq "^($h(:$h)*)?\$" || return 1
            done
            groups=0
            for side in "$left" "$right"; do
                [ -z "$side" ] || groups=$((groups + $(printf '%s' "$side" | tr -cd ':' | wc -c) + 1))
            done
            [ "$groups" -le 7 ] || return 1
            ;;
        *) printf '%s' "$1" | grep -Eq "^($h:){7}$h\$" || return 1 ;;
    esac
    return 0
}
port_ok() {
    printf '%s' "$1" | grep -Eq '^[1-9][0-9]{0,4}$' || return 1
    [ "$1" -le 65535 ]
}

RES_LIST=""
RES_COUNT=0
for t in $(printf '%s' "$RAW" | tr ',' ' '); do
    case "$t" in
        \[*\]:*)   a="${t%\]:*}"; a="${a#\[}"; p="${t##*\]:}"; ipv6_ok "$a" && port_ok "$p" || res_fail; e="[$a]:$p" ;;
        \[*\])     a="${t#\[}"; a="${a%\]}"; ipv6_ok "$a" || res_fail; e="[$a]" ;;
        *:*:*)     ipv6_ok "$t" || res_fail; e="[$t]" ;;
        *:*)       a="${t%%:*}"; p="${t##*:}"; ipv4_ok "$a" && port_ok "$p" || res_fail; e="$a:$p" ;;
        *)         ipv4_ok "$t" || res_fail; e="$t" ;;
    esac
    RES_LIST="$RES_LIST $e"
    RES_COUNT=$((RES_COUNT + 1))
done
[ "$RES_COUNT" -ge 1 ] || res_fail

# —— https upstreams verify the certificate ——
TLS=0
case "$BACKEND" in https://*) TLS=1 ;; esac
case "$AI" in https://*) TLS=1 ;; esac
if [ "$TLS" -eq 1 ] && [ ! -s "$CA_FILE" ]; then
    fail "an https upstream needs the CA bundle $CA_FILE, which is missing in this image"
fi

{
    echo "# generated by 16-xexam-upstreams.sh"
    echo "resolver${RES_LIST} valid=10s ipv6=off;"
    echo "map \$host \$xexam_backend_upstream { default \"$BACKEND\"; }"
    echo "map \$host \$xexam_ai_upstream { default \"$AI\"; }"
    if [ "$TLS" -eq 1 ]; then
        echo "proxy_ssl_server_name on;"
        echo "proxy_ssl_verify on;"
        echo "proxy_ssl_verify_depth 3;"
        echo "proxy_ssl_trusted_certificate $CA_FILE;"
    fi
} > "$OUT"
echo "xexam-upstreams: info: backend $BACKEND, ai-service $AI" >&2
