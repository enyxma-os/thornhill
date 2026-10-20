# thornhill.sh — RFC-001 §4.1: Kabuk hata kancası (sıfır maliyetli köprü)
#
# Kurallar:
#   - Yalnızca exit_code != 0 iken yazar (gürültü kontrolü, RFC-001 §4.1).
#   - fd 9 kapalıysa / daemon yoksa sessizce pas geçer (kabuk asla bloklanmaz).
#   - Sabit kullanıcı yolu YOK: soket $XDG_RUNTIME_DIR'den çözülür (ADR-062).
#
# Kurulum (zsh — .zshrc'ye):
#   source <thornhill-paket-yolu>/share/thornhill/shell-hook/thornhill.sh
#
# Not (zsh): precmd komut metnini argüman olarak ALMAZ; komutu preexec yakalar,
# precmd ise son komutun $status'ünü raporlar (yalnız ilk okuma geçerlidir).

__thornhill_connect() {
  local sock="${THORNHILL_SOCK:-${XDG_RUNTIME_DIR:-}/thornhill/thornhill.sock}"
  [[ -S "$sock" ]] || return 0
  exec 9>>"$sock" 2>/dev/null || true
}
__thornhill_connect

if [[ -n "$ZSH_VERSION" ]]; then
  autoload -Uz add-zsh-hook

  # preexec: çalıştırılan komut metnini yakalar (precmd $1 alamaz)
  __thornhill_preexec() {
    __THORNHILL_LAST_CMD="$1"
  }

  __thornhill_precmd() {
    local code=$?
    [[ $code -eq 0 ]] && return 0                      # gürültü kontrolü
    local cmd="$__THORNHILL_LAST_CMD"
    __THORNHILL_LAST_CMD=""
    [[ -n "$cmd" ]] || return 0
    # JSON'a güvenli gömme: ters bölü, tırnak, satır sonu temizliği + 200 bayt kesme
    local cmd_json
    cmd_json=$(print -r -- "$cmd" | head -c 200 \
      | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g' | tr '\n' ' ')
    printf '{"op":"event","req":"t-%s","src":"shell","kind":"cmd.done","subject":"%d","ctx":{"exit_code":%d,"cmd":"%s"}}\n' \
      "$EPOCHSECONDS" "$$" "$code" "$cmd_json" >&9 2>/dev/null || true
  }

  add-zsh-hook preexec __thornhill_preexec
  add-zsh-hook precmd __thornhill_precmd

elif [[ -n "$BASH_VERSION" ]]; then
  __thornhill_preexec_bash() {
    __THORNHILL_LAST_CMD="$1"
  }
  __thornhill_report_bash() {
    local code=$?
    [[ $code -eq 0 ]] && return 0
    local cmd="$__THORNHILL_LAST_CMD"
    __THORNHILL_LAST_CMD=""
    local cmd_json
    cmd_json=$(printf '%s' "$cmd" | head -c 200 \
      | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g' | tr '\n' ' ')
    printf '{"op":"event","req":"t-%s","src":"shell","kind":"cmd.done","subject":"%d","ctx":{"exit_code":%d,"cmd":"%s"}}\n' \
      "$EPOCHSECONDS" "$$" "$code" "$cmd_json" >&9 2>/dev/null || true
  }
  # bash 5+ preexec kancası (DEBUG sinyali yoluya); eski bash'te cmd boş kalır — sorun değil
  trap '__thornhill_preexec_bash "$BASH_COMMAND"' DEBUG
  [[ -n "$PROMPT_COMMAND" ]] \
    && PROMPT_COMMAND="__thornhill_report_bash;${PROMPT_COMMAND}" \
    || PROMPT_COMMAND="__thornhill_report_bash"
fi
