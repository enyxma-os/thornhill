# thornhill.sh — RFC-001 §4.1: Kabuk hata kancası (sıfır maliyetli köprü)
#
# v2 — ENXIO düzeltmesi (Seçenek 1 izleme turu):
#   Önceki sürüm `exec 9>>"$sock"` ile sokete doğrudan yönlendirme yapıyordu.
#   Bu YALNIZ zsh'ta çalışır (zsh, yönlendirmede socket() + connect() yapabilen
#   tek büyük kabuktur); bash/fish'te Linux VFS, SOCK_STREAM soketine open()
#   izin vermez ve ENXIO döner. Yeni desende her üç kabuk aynı yolu kullanır:
#   hata anında `thornhill-ctl inject` arka planda çalışır ( Soket yoksa 0 ms ).
#
# Kurallar:
#   - Yalnızca exit_code != 0 iken yazar.
#   - thornhill-ctl yolda yoksa veya soket yoksa sessizce pas geçer.
#   - Sabit kullanıcı yolu YOK: $XDG_RUNTIME_DIR üzerinden çözülür.

[[ -n "${__THORNHILL_HOOK_LOADED:-}" ]] && return 0
__THORNHILL_HOOK_LOADED=1

__thornhill_escape() {
  # JSON'a güvenli gömme: ters bölü + tırnak + satır sonu temizliği, 200 bayt kesme
  printf '%s' "$1" | head -c 200 \
    | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g' | tr '\n\r' '  '
}

__thornhill_report() {
  local code="$1" cmd="$2"
  [[ "$code" -ne 0 ]] || return 0
  [[ -n "$cmd" ]] || return 0
  command -v thornhill-ctl >/dev/null 2>&1 || return 0
  local sock="${THORNHILL_SOCK:-${XDG_RUNTIME_DIR:-}/thornhill/thornhill.sock}"
  [[ -S "$sock" ]] || return 0
  thornhill-ctl --socket "$sock" inject \
    --src shell --kind cmd.done --subject "$$" \
    --ctx "$(printf '{"exit_code": %d, "cmd": "%s"}' "$code" "$(__thornhill_escape "$cmd")")" \
    >/dev/null 2>&1 &
  disown 2>/dev/null || true
}

# --- zsh ---
if [[ -n "$ZSH_VERSION" ]]; then
  autoload -Uz add-zsh-hook
  __thornhill_preexec()  { __THORNHILL_LAST_CMD="$1"; }
  __thornhill_precmd()   {
    local code=$?
    __thornhill_report "$code" "$__THORNHILL_LAST_CMD"
    __THORNHILL_LAST_CMD=""
  }
  add-zsh-hook preexec __thornhill_preexec
  add-zsh-hook precmd  __thornhill_precmd

# --- bash ---
elif [[ -n "$BASH_VERSION" ]]; then
  __thornhill_preexec_bash() {
    case "$BASH_COMMAND" in
      *starship*|*__thornhill*|*__zoxide*|*PROMPT_COMMAND*) ;;
      *) __THORNHILL_LAST_CMD="$BASH_COMMAND" ;;
    esac
  }
  __thornhill_report_bash()  {
    local code=$?
    local cmd="$__THORNHILL_LAST_CMD"
    if [[ -z "$cmd" || "$cmd" == *"starship"* || "$cmd" == *"__thornhill"* || "$cmd" == *"__zoxide"* ]]; then
      cmd=$(fc -ln -1 2>/dev/null | sed -e 's/^[[:space:]]*//')
      [[ -n "$cmd" ]] || cmd=$(HISTTIMEFORMAT= history 1 2>/dev/null | sed -e 's/^[ ]*[0-9]*[ ]*//')
    fi
    __thornhill_report "$code" "$cmd"
    __THORNHILL_LAST_CMD=""
  }
  trap '__thornhill_preexec_bash' DEBUG
  [[ -n "$PROMPT_COMMAND" ]] \
    && PROMPT_COMMAND="__thornhill_report_bash;${PROMPT_COMMAND}" \
    || PROMPT_COMMAND="__thornhill_report_bash"
fi
