#!/usr/bin/env node
// TokenPlus 交互式配置器：选客户端 → 粘贴 API Key → 选模型 → 逐个配置。
// 零依赖；真正写配置的是仓库根目录的 <工具>-setup.sh，和单脚本模式是同一套逻辑。
import { spawn } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import readline from 'node:readline'
import { fileURLToPath } from 'node:url'

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const SITE = 'https://www.tokenplus.cn/'
const GATEWAY = (process.env.TOKENPLUS_GATEWAY || 'https://api.tokenplus.cn').replace(/\/$/, '')
const HOME = os.homedir()
const exists = (p) => fs.existsSync(p)
const onPath = (bin) =>
  (process.env.PATH || '').split(path.delimiter).some((dir) => dir && exists(path.join(dir, bin)))

const TOOLS = [
  { id: 'claude', name: 'Claude Code', model: true,
    found: () => onPath('claude') || exists(path.join(HOME, '.claude')) },
  { id: 'claude-desktop', name: 'Claude 桌面版', model: false,
    found: () => exists('/Applications/Claude.app') || exists(path.join(HOME, 'Applications/Claude.app')) },
  { id: 'codex', name: 'Codex', model: true,
    found: () => onPath('codex') || exists('/Applications/Codex.app') || exists(path.join(HOME, '.codex')) },
  { id: 'grok', name: 'Grok Build', model: true,
    found: () => onPath('grok') || exists(path.join(HOME, '.grok')) },
  { id: 'deepseek', name: 'DeepSeek Harness', model: true,
    found: () => exists('/Applications/DeepSeek Harness.app') || exists(path.join(HOME, 'Applications/DeepSeek Harness.app')) },
  { id: 'workbuddy', name: 'WorkBuddy', model: false,
    found: () => exists('/Applications/WorkBuddy.app') || exists('/Applications/WorkBuddy AI.app') || exists(path.join(HOME, '.workbuddy')) },
]

// ---------- 主题 ----------
const out = process.stdout
const COLOR = out.isTTY && !process.env.NO_COLOR
const sgr = (code) => (s) => (COLOR ? `\x1b[${code}m${s}\x1b[0m` : s)
const rgb = (r, g, b) => sgr(`38;2;${r};${g};${b}`)
const violet = rgb(167, 139, 250)
const cyan = rgb(103, 232, 249)
const green = rgb(74, 222, 128)
const yellow = rgb(250, 204, 21)
const red = rgb(248, 113, 113)
const dim = rgb(113, 113, 130)
const bold = sgr('1')
const BAR = dim('│')
const FROM = [139, 92, 246]
const TO = [34, 211, 238]
const gradient = (text) => {
  if (!COLOR) return text
  const chars = [...text]
  return chars
    .map((ch, i) => {
      const t = chars.length > 1 ? i / (chars.length - 1) : 0
      const c = FROM.map((from, k) => Math.round(from + (TO[k] - from) * t))
      return `\x1b[1;38;2;${c[0]};${c[1]};${c[2]}m${ch}`
    })
    .join('') + '\x1b[0m'
}
const width = (s) => [...s].reduce((n, ch) => n + (ch.codePointAt(0) > 0x2e7f ? 2 : 1), 0)
const pad = (s, n) => s + ' '.repeat(Math.max(0, n - width(s)))
const maskKey = (key) => `sk-…${key.slice(-4)}`

function banner() {
  out.write('\n')
  out.write(`   ${gradient('▀█▀ █▀█ █▄▀ █▀▀ █▄ █ █▀█ █   █ █ █▀▀')}\n`)
  out.write(`   ${gradient(' █  █▄█ █ █ ██▄ █ ▀█ █▀▀ █▄▄ █▄█ ▄██')}\n`)
  out.write(`   ${dim('一条命令，把 AI 编程工具接入 TokenPlus')}\n\n`)
  out.write(`${dim('┌')}  ${bold('tokenplus setup')}\n${BAR}\n`)
}

// ---------- 交互组件 ----------
class Cancelled extends Error {}

// 整个交互期间保持 raw 模式：提示之间敲的键不会被终端回显出来
let rawActive = false
let prompting = false
function rawBegin() {
  readline.emitKeypressEvents(process.stdin)
  process.stdin.setRawMode(true)
  process.stdin.resume()
  process.stdin.on('keypress', (_, key = {}) => {
    if (key.ctrl && key.name === 'c' && !prompting) {
      rawEnd()
      out.write(`\r\x1b[K\x1b[?25h${dim('└')}  ${dim('已取消')}\n\n`)
      process.exit(130)
    }
  })
  rawActive = true
}
function rawEnd() {
  if (!rawActive) return
  rawActive = false
  process.stdin.setRawMode(false)
  process.stdin.pause()
}
const isEnter = (key) => key.name === 'return' || key.name === 'enter'

// 反复重绘一块区域，直到 onKey 返回结果
function prompt(render, onKey) {
  return new Promise((resolve, reject) => {
    let drawn = 0
    const draw = () => {
      const lines = render()
      if (drawn) out.write(`\x1b[${drawn}A`)
      out.write('\r\x1b[J' + lines.join('\n') + '\n')
      drawn = lines.length
    }
    const finish = (fn, value) => {
      process.stdin.off('keypress', handler)
      prompting = false
      out.write(`\x1b[${drawn}A\r\x1b[J\x1b[?25h`)
      fn(value)
    }
    const handler = (str, key = {}) => {
      if (key.ctrl && key.name === 'c') return finish(reject, new Cancelled())
      const result = onKey(str, key)
      if (result !== undefined) return finish(resolve, result)
      draw()
    }
    prompting = true
    process.stdin.on('keypress', handler)
    out.write('\x1b[?25l')
    draw()
  })
}

const done = (title, value) => out.write(`${green('◇')}  ${title}\n${BAR}  ${dim(value)}\n${BAR}\n`)

async function multiselect(title, items) {
  let cursor = 0
  const picked = items.map((item) => item.picked)
  const result = await prompt(
    () => [
      `${violet('◆')}  ${title}  ${dim('↑↓ 移动 · 空格勾选 · a 全选 · 回车确认')}`,
      ...items.map((item, i) => {
        const box = picked[i] ? violet('◼') : dim('◻')
        const label = i === cursor ? bold(pad(item.label, 18)) : pad(item.label, 18)
        return `${violet('│')}  ${i === cursor ? violet('❯') : ' '} ${box} ${label} ${item.hint}`
      }),
      violet('└'),
    ],
    (str, key) => {
      if (key.name === 'up' || str === 'k') cursor = (cursor + items.length - 1) % items.length
      else if (key.name === 'down' || str === 'j') cursor = (cursor + 1) % items.length
      else if (key.name === 'space') picked[cursor] = !picked[cursor]
      else if (str === 'a' || str === 'A') {
        const all = picked.every(Boolean)
        picked.fill(!all)
      } else if (isEnter(key)) return items.filter((_, i) => picked[i])
    }
  )
  return result
}

async function password(title, hint, error) {
  let value = ''
  return prompt(
    () => [
      `${violet('◆')}  ${title}  ${dim(hint)}`,
      `${violet('│')}  ${value ? '▪'.repeat(Math.min(value.length, 48)) : dim('粘贴后回车')}${violet('▏')}`,
      error ? `${violet('└')}  ${yellow(error)}` : violet('└'),
    ],
    (str, key) => {
      if (isEnter(key)) return value.replace(/\s+/g, '')
      if (key.name === 'backspace') value = value.slice(0, -1)
      else if (key.ctrl && key.name === 'u') value = ''
      else if (str && !key.ctrl && !key.meta && str >= ' ') value += str
    }
  )
}

async function search(title, options) {
  let query = ''
  let cursor = 0
  let top = 0
  const ROWS = 10
  const filtered = () => {
    const q = query.toLowerCase()
    return options.filter((option) => option.toLowerCase().includes(q))
  }
  return prompt(
    () => {
      const list = filtered()
      cursor = Math.min(cursor, Math.max(0, list.length - 1))
      if (cursor < top) top = cursor
      if (cursor >= top + ROWS) top = cursor - ROWS + 1
      const rows = list.slice(top, top + ROWS).map((option, i) =>
        top + i === cursor
          ? `${violet('│')}  ${violet('❯')} ${bold(violet(option))}`
          : `${violet('│')}    ${option}`
      )
      return [
        `${violet('◆')}  ${title}  ${dim('↑↓ 移动 · 打字筛选 · 回车确认')}`,
        `${violet('│')}  ${dim('筛选')} ${query}${violet('▏')}`,
        ...(rows.length ? rows : [`${violet('│')}    ${dim('没有匹配的模型')}`]),
        `${violet('└')}  ${dim(`${list.length} / ${options.length} 个模型`)}`,
      ]
    },
    (str, key) => {
      const list = filtered()
      if (key.name === 'up') cursor = Math.max(0, cursor - 1)
      else if (key.name === 'down') cursor = Math.min(list.length - 1, cursor + 1)
      else if (isEnter(key)) {
        if (list.length) return list[cursor]
      } else if (key.name === 'backspace') {
        query = query.slice(0, -1)
        cursor = top = 0
      } else if (str && !key.ctrl && !key.meta && str >= ' ') {
        query += str
        cursor = top = 0
      }
    }
  )
}

function spinner(text) {
  if (!out.isTTY) {
    out.write(`${text}\n`)
    return () => {}
  }
  const frames = ['◒', '◐', '◓', '◑']
  let i = 0
  out.write('\x1b[?25l')
  const tick = () => out.write(`\r\x1b[K${violet(frames[i++ % frames.length])}  ${text}`)
  tick()
  const timer = setInterval(tick, 90)
  return () => {
    clearInterval(timer)
    out.write('\r\x1b[K\x1b[?25h')
  }
}

// ---------- TokenPlus ----------
async function fetchModels(key) {
  let response
  try {
    response = await fetch(`${GATEWAY}/v1/models`, {
      headers: { Authorization: `Bearer ${key}` },
      signal: AbortSignal.timeout(25000),
    })
  } catch {
    throw new Error('连不上 TokenPlus 网关，请检查网络后重试')
  }
  if (response.status === 401 || response.status === 403) return null
  if (!response.ok) throw new Error(`TokenPlus 网关返回了 HTTP ${response.status}`)
  const body = await response.json().catch(() => ({}))
  const ids = [...new Set((body.data ?? []).map((item) => item?.id))].filter(
    (id) => typeof id === 'string' && /^[A-Za-z0-9._:/-]+$/.test(id)
  )
  if (!ids.length) throw new Error('没有读取到可用模型列表')
  return ids
}

const keyValid = (key) => /^sk-[A-Za-z0-9_-]+$/.test(key)

function runTool(tool, env) {
  return new Promise((resolve) => {
    const child = spawn('bash', [path.join(ROOT, `${tool.id}-setup.sh`)], {
      env: { ...process.env, ...env, TOKENPLUS_EMBED: '1', TOKENPLUS_ASSET_DIR: ROOT },
      stdio: ['ignore', 'pipe', 'pipe'],
    })
    let text = ''
    child.stdout.on('data', (chunk) => (text += chunk))
    child.stderr.on('data', (chunk) => (text += chunk))
    child.on('error', (error) => resolve({ ok: false, lines: [error.message] }))
    child.on('close', (code) => {
      const lines = text
        .replace(/\x1b\[[0-9;?]*[A-Za-z]/g, '')
        .split('\n')
        .map((line) => line.trim())
        .filter(Boolean)
      resolve({ ok: code === 0, lines })
    })
  })
}

// ---------- 参数 ----------
function parseArgs(argv) {
  const args = { tools: null, key: process.env.TOKENPLUS_API_KEY || '', model: process.env.TOKENPLUS_MODEL || '', open: true }
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i]
    const next = () => argv[++i] ?? ''
    if (arg === '-k' || arg === '--key') args.key = next()
    else if (arg === '-m' || arg === '--model') args.model = next()
    else if (arg === '-t' || arg === '--tools') args.tools = next().split(',').map((s) => s.trim()).filter(Boolean)
    else if (arg === '--no-open') args.open = false
    else if (arg === '-h' || arg === '--help') args.help = true
    else if (arg === '-v' || arg === '--version') args.version = true
    else throw new Error(`不认识的参数：${arg}（--help 查看用法）`)
  }
  return args
}

const HELP = `用法：npx github:pengcai6/tokenplus-helper-release [选项]

不带选项时全程交互：选客户端 → 粘贴 API Key → 选模型 → 自动配置。

选项：
  -t, --tools <列表>   要配置的客户端，逗号分隔：${TOOLS.map((tool) => tool.id).join(',')}
  -k, --key <key>      TokenPlus API Key（也可用环境变量 TOKENPLUS_API_KEY）
  -m, --model <模型>   要使用的模型
      --no-open        不自动打开 TokenPlus 网站
  -h, --help           显示帮助
  -v, --version        显示版本
`

async function main() {
  const args = parseArgs(process.argv.slice(2))
  if (args.help) return void out.write(HELP)
  if (args.version) {
    return void out.write(`${JSON.parse(fs.readFileSync(path.join(ROOT, 'package.json'), 'utf8')).version}\n`)
  }
  if (process.platform !== 'darwin') throw new Error('目前只支持 macOS')
  const interactive = Boolean(process.stdin.isTTY && out.isTTY)
  banner()
  if (interactive) rawBegin()

  // 1 · 选客户端
  let tools
  if (args.tools) {
    const unknown = args.tools.filter((id) => !TOOLS.some((tool) => tool.id === id))
    if (unknown.length) throw new Error(`不认识的客户端：${unknown.join(', ')}`)
    tools = TOOLS.filter((tool) => args.tools.includes(tool.id))
  } else if (interactive) {
    const picked = await multiselect(
      '要配置哪些客户端？',
      TOOLS.map((tool) => {
        const found = tool.found()
        return { tool, label: tool.name, picked: found, hint: found ? green('已安装') : dim('未检测到') }
      })
    )
    tools = picked.map((item) => item.tool)
  } else {
    tools = TOOLS.filter((tool) => tool.found())
  }
  if (!tools.length) {
    out.write(`${dim('└')}  ${yellow('没有选择任何客户端，已退出')}\n\n`)
    return
  }
  done('要配置的客户端', tools.map((tool) => tool.name).join('、'))

  // 2 · API Key
  let key = args.key.trim()
  let models = null
  if (key) {
    if (!keyValid(key)) throw new Error('API Key 格式无效，应以 sk- 开头')
    const stop = spinner('正在验证 API Key')
    try {
      models = await fetchModels(key)
    } finally {
      stop()
    }
    if (!models) throw new Error('API Key 无效、已停用或已过期')
  } else {
    if (!interactive) throw new Error('没有可交互的终端，请用 --key 传入 API Key')
    if (args.open) spawn('/usr/bin/open', [SITE], { stdio: 'ignore' }).on('error', () => {})
    let error = ''
    for (let tries = 0; !models; tries++) {
      if (tries >= 5) throw new Error('多次输入无效，已退出')
      key = await password(
        '粘贴 TokenPlus API Key',
        args.open ? '已在浏览器打开 tokenplus.cn，登录后创建或复制一个 Key' : `在 ${SITE} 登录后创建或复制一个 Key`,
        error
      )
      if (!keyValid(key)) {
        error = 'API Key 格式无效，应以 sk- 开头，请重新粘贴'
        continue
      }
      const stop = spinner('正在验证 API Key')
      try {
        models = await fetchModels(key)
      } finally {
        stop()
      }
      if (!models) error = 'API Key 无效、已停用或已过期，请重新粘贴'
    }
  }
  done('API Key', `${maskKey(key)} · ${models.length} 个模型可用`)

  // 3 · 模型
  let model = args.model.trim()
  if (tools.some((tool) => tool.model)) {
    if (model) {
      if (!models.includes(model)) throw new Error(`模型 ${model} 不在这个 API Key 可用的列表里`)
    } else {
      if (!interactive) throw new Error('没有可交互的终端，请用 --model 指定模型')
      model = await search('选择模型', models)
    }
    done('模型', model)
  }

  // 4 · 逐个配置
  const env = {
    TOKENPLUS_API_KEY: key,
    TOKENPLUS_MODEL: model,
    TOKENPLUS_MODELS: models.join('\n'),
    TOKENPLUS_GATEWAY: GATEWAY,
  }
  const failed = []
  for (const tool of tools) {
    const stop = spinner(`正在配置 ${tool.name}`)
    const result = await runTool(tool, env)
    stop()
    if (result.ok) {
      out.write(`${green('◇')}  ${tool.name}  ${green('已配置')}\n`)
      for (const line of result.lines) {
        if (line.startsWith('[注意]')) out.write(`${BAR}  ${yellow(line)}\n`)
        else if (/^\[完成\] (已写入|连接正常)/.test(line)) out.write(`${BAR}  ${dim(line.replace('[完成] ', ''))}\n`)
      }
    } else {
      failed.push(tool.name)
      out.write(`${red('■')}  ${tool.name}  ${red('未完成')}\n`)
      for (const line of result.lines.slice(-6)) out.write(`${BAR}  ${line.startsWith('[失败]') ? red(line) : dim(line)}\n`)
    }
    out.write(`${BAR}\n`)
  }

  const okCount = tools.length - failed.length
  if (failed.length) {
    out.write(`${dim('└')}  ${yellow(`${okCount} 个已配置，${failed.length} 个未完成：${failed.join('、')}`)}\n\n`)
    process.exitCode = 1
  } else {
    out.write(`${dim('└')}  ${gradient(`完成，${okCount} 个客户端已接入 TokenPlus`)}  ${dim('重启对应的应用或新开终端后生效')}\n\n`)
  }
}

main().then(rawEnd).catch((error) => {
  rawEnd()
  if (out.isTTY) out.write('\x1b[?25h')
  if (error instanceof Cancelled) {
    out.write(`${dim('└')}  ${dim('已取消')}\n\n`)
    process.exitCode = 130
    return
  }
  out.write(`${dim('└')}  ${red(error.message)}\n\n`)
  process.exitCode = 1
})
