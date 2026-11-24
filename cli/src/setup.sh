# id|名称|是否需要选模型|对齐用的空格
TP_TOOLS='claude|Claude Code|1|     
claude-desktop|Claude 桌面版|0|   
codex|Codex|1|           
grok|Grok Build|1|      
deepseek|DeepSeek Harness|1|
workbuddy|WorkBuddy|0|       '

tp_tool_installed() {
  case "$1" in
    claude) command -v claude >/dev/null 2>&1 || [ -d "$HOME/.claude" ] ;;
    claude-desktop) [ -d '/Applications/Claude.app' ] || [ -d "$HOME/Applications/Claude.app" ] ;;
    codex) command -v codex >/dev/null 2>&1 || [ -d '/Applications/Codex.app' ] || [ -d "$HOME/.codex" ] ;;
    grok) command -v grok >/dev/null 2>&1 || [ -d "$HOME/.grok" ] ;;
    deepseek) [ -d '/Applications/DeepSeek Harness.app' ] || [ -d "$HOME/Applications/DeepSeek Harness.app" ] ;;
    workbuddy) [ -d '/Applications/WorkBuddy.app' ] || [ -d '/Applications/WorkBuddy AI.app' ] || [ -d "$HOME/.workbuddy" ] ;;
  esac
}

printf '\n'
printf '\033[1;38;5;141m   ▀█▀ █▀█ █▄▀ █▀▀ █▄ █ \033[38;5;111m█▀█ █   █ █ █▀▀\033[0m\n'
printf '\033[1;38;5;135m    █  █▄█ █ █ ██▄ █ ▀█ \033[38;5;75m█▀▀ █▄▄ █▄█ ▄██\033[0m\n'
printf '\033[38;5;245m   一条命令，把 AI 编程工具接入 TokenPlus\033[0m\n\n'

ids=() names=() needs=() pads=() found=() picked=()
while IFS='|' read -r t_id t_name t_need t_pad; do
  [ -n "$t_id" ] || continue
  ids[${#ids[@]}]="$t_id"; names[${#names[@]}]="$t_name"; needs[${#needs[@]}]="$t_need"; pads[${#pads[@]}]="$t_pad"
  if tp_tool_installed "$t_id"; then found[${#found[@]}]=1; picked[${#picked[@]}]=1
  else found[${#found[@]}]=0; picked[${#picked[@]}]=0; fi
done <<EOT
$TP_TOOLS
EOT
count=${#ids[@]}

if [ -n "${TOKENPLUS_TOOLS:-}" ]; then
  i=0
  while [ "$i" -lt "$count" ]; do
    case ",$TOKENPLUS_TOOLS," in *",${ids[$i]},"*) picked[$i]=1 ;; *) picked[$i]=0 ;; esac
    i=$((i + 1))
  done
elif tp_has_tty; then
  # 多选：↑↓ 移动，空格勾选，a 全选/全不选，回车确认
  cursor=0 drawn=0
  exec 3>/dev/tty
  tp_ui_begin
  while :; do
    [ "$drawn" -gt 0 ] && printf '\033[%dA' "$drawn" >&3
    printf '\r\033[J' >&3
    printf '\033[38;5;141m  ?\033[0m 要配置哪些客户端 \033[38;5;245m（↑↓ 移动 · 空格勾选 · a 全选 · 回车确认）\033[0m\n' >&3
    i=0
    while [ "$i" -lt "$count" ]; do
      if [ "${picked[$i]}" = 1 ]; then box='\033[38;5;141m◼\033[0m'; else box='\033[38;5;245m◻\033[0m'; fi
      if [ "${found[$i]}" = 1 ]; then tag='\033[38;5;78m已安装\033[0m'; else tag='\033[38;5;245m未检测到\033[0m'; fi
      if [ "$i" = "$cursor" ]; then ptr='\033[38;5;141m❯\033[0m'; else ptr=' '; fi
      printf "  $ptr $box %s%s  $tag\n" "${names[$i]}" "${pads[$i]}" >&3
      i=$((i + 1))
    done
    drawn=$((count + 1))
    IFS= read -rsn1 key </dev/tty || key=''
    case "$key" in
      '') break ;;
      ' ') picked[$cursor]=$((1 - ${picked[$cursor]})) ;;
      a | A)
        all_on=1; i=0
        while [ "$i" -lt "$count" ]; do [ "${picked[$i]}" = 1 ] || all_on=0; i=$((i + 1)); done
        i=0
        while [ "$i" -lt "$count" ]; do picked[$i]=$((1 - all_on)); i=$((i + 1)); done
        ;;
      k) [ "$cursor" -gt 0 ] && cursor=$((cursor - 1)) ;;
      j) [ "$cursor" -lt $((count - 1)) ] && cursor=$((cursor + 1)) ;;
      $'\033')
        rest=''
        IFS= read -rsn2 -t 1 rest </dev/tty || rest=''
        case "$rest" in
          '[A') [ "$cursor" -gt 0 ] && cursor=$((cursor - 1)) ;;
          '[B') [ "$cursor" -lt $((count - 1)) ] && cursor=$((cursor + 1)) ;;
        esac
        ;;
    esac
  done
  printf '\033[%dA\r\033[J' "$drawn" >&3
  exec 3>&-
  tp_ui_end
  trap - EXIT INT TERM
fi

need_model=0 chosen=''
i=0
while [ "$i" -lt "$count" ]; do
  if [ "${picked[$i]}" = 1 ]; then
    chosen="$chosen${chosen:+、}${names[$i]}"
    [ "${needs[$i]}" = 1 ] && need_model=1
  fi
  i=$((i + 1))
done
[ -n "$chosen" ] || { yellow '没有选择任何客户端，已退出'; exit 0; }
green "将配置：$chosen"

tp_require_key
green 'API Key 有效'
if [ "$need_model" = 1 ]; then tp_require_model; fi

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
ok_list='' fail_list=''
i=0
while [ "$i" -lt "$count" ]; do
  if [ "${picked[$i]}" = 1 ]; then
    printf '\n\033[1;38;5;141m  ◆ %s\033[0m\n' "${names[$i]}"
    script="$work/${ids[$i]}-setup.sh"
    if tp_fetch_asset "${ids[$i]}-setup.sh" "$script" \
      && TOKENPLUS_API_KEY="$API_KEY" TOKENPLUS_MODEL="$MODEL" TOKENPLUS_MODELS="$MODELS" \
        TOKENPLUS_GATEWAY="$TP_GATEWAY" TOKENPLUS_EMBED=1 bash "$script" </dev/null; then
      ok_list="$ok_list${ok_list:+、}${names[$i]}"
    else
      fail_list="$fail_list${fail_list:+、}${names[$i]}"
    fi
  fi
  i=$((i + 1))
done

printf '\n\033[38;5;240m  ─────────────────────────────────────────────\033[0m\n'
[ -n "$ok_list" ] && green "已配置：$ok_list"
[ -n "$fail_list" ] && red "未完成：$fail_list（往上翻可以看到原因）"
[ -z "$fail_list" ]
