import { randomUUID } from 'node:crypto'
/* Copyright (C) 2023-2026 QuantumNous
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Adapted for TokenPlus: the setup-code exchange is replaced by an API key and
 * model list passed in through the environment by workbuddy-setup.sh.
 */
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { pathToFileURL } from 'node:url'

const PROVIDERS = new Set(['tokenplus'])
const MANAGED_TAG = 'tokenplus-helper'

class WorkBuddySetupError extends Error {}

export function validateWorkBuddyOrigin(value) {
  const url = new URL(value)
  if (
    (url.protocol !== 'https:' &&
      !(
        url.protocol === 'http:' &&
        ['localhost', '127.0.0.1', '[::1]'].includes(url.hostname)
      )) ||
    url.username ||
    url.password ||
    url.pathname !== '/' ||
    url.search ||
    url.hash
  ) {
    throw new WorkBuddySetupError(
      '站点地址必须使用 HTTPS（本机地址除外），且不能包含账号密码或路径'
    )
  }
  return url.origin
}

export function resolveWorkBuddyConfigDir(
  home = os.homedir(),
  env = process.env
) {
  const override =
    env.WORKBUDDY_CONFIG_DIR?.trim() || env.CODEBUDDY_CONFIG_DIR?.trim()
  if (override) return path.resolve(override)
  const domestic = path.join(home, '.workbuddy')
  const international = path.join(home, '.workbuddy-ai')
  return !fs.existsSync(domestic) && fs.existsSync(international)
    ? international
    : domestic
}

/** WorkBuddy 5.9.3 accepts both a model array and { models, availableModels }.
 * Model IDs are the actual upstream names; never namespace them or send a
 * provider ID as the request's model. WorkBuddy 5.9.3 passes the model's `api`
 * through to its bundled CLI, including native OpenAI Responses support.
 * Each family has its own gateway token and protocol.
 */
export function applyWorkBuddyConfiguration(config, configDir) {
  const providers = config?.providers
  if (
    !Array.isArray(providers) ||
    providers.length !== 1
  ) {
    throw new WorkBuddySetupError(
      'WorkBuddy 配置不完整，未修改配置'
    )
  }
  const added = []
  for (const provider of providers) {
    if (
      !PROVIDERS.has(provider?.provider_id) ||
      !['openai-responses', 'openai-completions'].includes(provider.api) ||
      typeof provider.api_key !== 'string' ||
      !/^sk-[A-Za-z0-9_-]+$/.test(provider.api_key) ||
      typeof provider.provider_name !== 'string' ||
      !Array.isArray(provider.models) ||
      !provider.models.length ||
      !provider.models.includes(provider.model) ||
      !provider.models.every(
        (id) =>
          typeof id === 'string' &&
          id.trim() === id &&
          id.length > 0 &&
          !/\p{Cc}/u.test(id)
      )
    ) {
      throw new WorkBuddySetupError(
        'WorkBuddy 配置不完整，未修改配置'
      )
    }
    const endpoint = new URL(provider.base_url)
    validateWorkBuddyOrigin(endpoint.origin)
    if (
      endpoint.username ||
      endpoint.password ||
      endpoint.pathname !== '/v1' ||
      endpoint.search ||
      endpoint.hash
    ) {
      throw new WorkBuddySetupError('接口地址无效')
    }
    for (const id of provider.models) {
      added.push({
        id,
        name: id,
        vendor: provider.provider_name,
        api: provider.api,
        url:
          provider.api === 'openai-responses'
            ? `${endpoint.origin}/v1`
            : `${endpoint.origin}/v1/chat/completions`,
        apiKey: provider.api_key,
        maxInputTokens: 32768,
        maxOutputTokens: 4096,
        supportsToolCall: true,
        tags: ['custom', MANAGED_TAG, provider.provider_id],
      })
    }
  }
  const ids = added.map((model) => model.id)
  if (new Set(ids).size !== ids.length) {
    throw new WorkBuddySetupError(
      '模型列表里有重复的名字，未修改配置'
    )
  }
  const file = path.join(configDir, 'models.json')
  const exists = fs.existsSync(file)
  const original = exists ? fs.readFileSync(file, 'utf8') : null
  let current = []
  if (exists) {
    try {
      current = JSON.parse(original.replace(/^\uFEFF/, ''))
    } catch {
      throw new WorkBuddySetupError(
        '现有 models.json 不是有效 JSON，未修改配置'
      )
    }
  }
  if (
    !current ||
    typeof current !== 'object' ||
    (!Array.isArray(current) &&
      ((current.models !== undefined && !Array.isArray(current.models)) ||
        (current.availableModels !== undefined &&
          (!Array.isArray(current.availableModels) ||
            !current.availableModels.every((id) => typeof id === 'string')))))
  ) {
    throw new WorkBuddySetupError('现有 models.json 格式不兼容，未修改配置')
  }
  const models = Array.isArray(current) ? current : (current.models ?? [])
  const retained = models.filter((model) => !model?.tags?.includes(MANAGED_TAG))
  if (retained.some((model) => ids.includes(model?.id))) {
    throw new WorkBuddySetupError(
      '已有同名自定义模型，未覆盖原配置；请先在 WorkBuddy 设置中重命名或移除同名模型后重试'
    )
  }
  const merged = [...retained, ...added]
  let output = merged
  if (!Array.isArray(current)) {
    output = { ...current, models: merged }
    // An absent/empty filter means "show all". Preserve that meaning and keep
    // all existing selections when the user has explicitly restricted the list.
    if (current.availableModels?.length) {
      const removed = new Set(
        models
          .filter((model) => model?.tags?.includes(MANAGED_TAG))
          .map((model) => model.id)
      )
      output.availableModels = [
        ...new Set([
          ...current.availableModels.filter((id) => !removed.has(id)),
          ...ids,
        ]),
      ]
    }
  }
  fs.mkdirSync(configDir, { recursive: true, mode: 0o700 })
  const stamp = `${Date.now()}-${randomUUID()}`
  const temp = `${file}.tokenplus-${stamp}.tmp`
  try {
    fs.writeFileSync(temp, `${JSON.stringify(output, null, 2)}\n`, {
      flag: 'wx',
      mode: 0o600,
    })
    if (exists) {
      fs.writeFileSync(`${file}.bak-${stamp}`, original, {
        flag: 'wx',
        mode: 0o600,
      })
    }
    if (
      fs.existsSync(file) !== exists ||
      (exists && fs.readFileSync(file, 'utf8') !== original)
    ) {
      throw new WorkBuddySetupError(
        '配置文件刚被 WorkBuddy 修改，请完全退出应用后重试'
      )
    }
    fs.renameSync(temp, file)
    if (process.platform !== 'win32') fs.chmodSync(file, 0o600)
  } finally {
    if (fs.existsSync(temp)) fs.unlinkSync(temp)
  }
  return { file, count: added.length }
}

export async function configureWorkBuddy(
  env = process.env,
  configDir = resolveWorkBuddyConfigDir()
) {
  const models = (env.TOKENPLUS_MODELS ?? '')
    .split('\n')
    .map((id) => id.trim())
    .filter(Boolean)
  const applied = applyWorkBuddyConfiguration(
    {
      providers: [
        {
          provider_id: 'tokenplus',
          provider_name: 'TokenPlus',
          api: 'openai-completions',
          api_key: env.TOKENPLUS_API_KEY ?? '',
          base_url: env.TOKENPLUS_BASE_URL || 'https://api.tokenplus.cn/v1',
          model: models[0],
          models,
        },
      ],
    },
    configDir
  )
  process.stdout.write(
    `  已写入 ${applied.count} 个模型：${applied.file}\n`
  )
  return applied
}

if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(fs.realpathSync(process.argv[1])).href
) {
  configureWorkBuddy().catch((error) => {
    // Do not print raw network errors, response bodies or credentials.
    const message =
      error instanceof WorkBuddySetupError
        ? error.message
        : 'WorkBuddy 配置未完成，请检查现有 models.json 后重试。'
    process.stderr.write(`${message}\n`)
    process.exitCode = 1
  })
}
