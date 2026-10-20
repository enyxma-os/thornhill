# shell-hook/ — Kabuk Köprüsü

Kabuk çıkış kodlarını duyu katmanına bağlar (RFC-001 §4.1).

## zsh (precmd)

```zsh
autoload -Uz add-zsh-hook
thornhill_report() {
  local code=$?
  [[ $code -ne 0 ]] && print -r -- \
    "{\"op\":\"event\",\"req\":\"$EPOCHSECONDS\",\"src\":\"shell\",\"kind\":\"cmd.done\",\"subject\":\"$$\",\"ctx\":{\"exit_code\":$code,\"cmd\":\"${1}\"}}" \
    >&9
}
add-zsh-hook precmd thornhill_report
# fd 9, daemon UDS'sine yönlendirilir (exec 9<>$XDG_RUNTIME_DIR/thornhill/thornhill.sock)
```

## bash (PROMPT_COMMAND)

```bash
__thornhill_report() {
  local code=$?
  [[ $code -ne 0 ]] && printf '{"op":"event","src":"shell","kind":"cmd.done","ctx":{"exit_code":%d}}\n' "$code" >&9
}
PROMPT_COMMAND="__thornhill_report;${PROMPT_COMMAND}"
```

Not: `precmd` içinde `$status`/`$?` son komutun kodunu taşır (yalnız ilk okuma);
hook yalnız `exit_code != 0` iken yazar (gürültü kontrolü).
