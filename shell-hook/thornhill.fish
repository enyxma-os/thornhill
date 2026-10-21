# thornhill.fish — RFC-001 §4.1: Fish kabuk hata kancası (v2, ENXIO-düzeltmeli)
#
# Fish, fd yönlendirmesiyle UNIX soketine bağlanamaz (bash ile aynı çekirdek
# kısıtı: SOCK_STREAM'e open() → ENXIO). Bu yüzden köprü, hata anında arka
# planda `thornhill-ctl inject` çalıştırır; soket/araç yoksa 0 ms'de pas geçer.
#
# Kurulum: services.thornhill.enableShellHook = true (Home Manager modülü
# programs.fish.interactiveShellInit içine otomatik source eder).

function __thornhill_escape
    # JSON'a güvenli gömme: ters bölü + tırnak + satır sonları, 200 bayt kesme
    printf '%s' "$argv[1]" | head -c 200 \
        | string replace -a '\\' '\\\\' \
        | string replace -a '"' '\\"' \
        | string replace -a -r '[\n\r]' ' ' \
        | string collect
end

function __thornhill_report --description "Thornhill hata raporu (arka plan, sessiz)"
    set -l code $argv[1]
    set -l cmd $argv[2]
    test "$code" -eq 0; and return 0
    test -n "$cmd"; or return 0
    command -q thornhill-ctl; or return 0
    set -l sock (set -q THORNHILL_SOCK; and echo $THORNHILL_SOCK; or echo "$XDG_RUNTIME_DIR/thornhill/thornhill.sock")
    test -S "$sock"; or return 0
    set -l cmd_json (__thornhill_escape "$cmd")
    command thornhill-ctl --socket "$sock" inject \
        --src shell --kind cmd.done --subject $fish_pid \
        --ctx (printf '{"exit_code": %d, "cmd": "%s"}' "$code" "$cmd_json") \
        >/dev/null 2>&1 &
    disown
end

function __thornhill_preexec --on-event fish_preexec
    set -g __thornhill_last_cmd "$argv[1]"
end

function __thornhill_postexec --on-event fish_postexec
    set -l code $status
    __thornhill_report "$code" "$__thornhill_last_cmd"
    set -e __thornhill_last_cmd
end
