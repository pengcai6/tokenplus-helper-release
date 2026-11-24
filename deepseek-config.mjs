/*
Copyright (C) 2023-2026 QuantumNous

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
GNU Affero General Public License for more details.

You should have received a copy of the GNU Affero General Public License
along with this program. If not, see <https://www.gnu.org/licenses/>.

For commercial licensing, please contact support@quantumnous.com
*/
// Adapted for TokenPlus: the one-time setup code exchange is replaced by an
// API key and model passed in through the environment by deepseek-setup.sh.
// The YAML merge, backup and atomic-write logic is unchanged.
// Shared by the Windows and macOS launchers. Use the desktop app's own YAML
// parser and writer locks so existing profiles, providers and credentials stay intact.
import fs from 'node:fs'
import { createRequire } from 'node:module'
import os from 'node:os'
import path from 'node:path'
import { pathToFileURL } from 'node:url'

const PROVIDER_ID = 'tokenplus'
const CREDENTIAL_REF = 'TOKENPLUS_API_KEY'

export function applyDeepSeekConfiguration(config, home, yaml) {
  const providers = config.providers
  const defaultProviderId = config.default_provider_id ?? providers?.[0]?.provider_id
  if (
    !Array.isArray(providers) ||
    providers.length !== 1 ||
    providers[0].provider_id !== PROVIDER_ID ||
    defaultProviderId !== PROVIDER_ID ||
    !providers.every(
      (provider) =>
        typeof provider.api_key === 'string' &&
        /^sk-[A-Za-z0-9_-]+$/.test(provider.api_key) &&
        provider.models?.length &&
        provider.models.every((id) => typeof id === 'string' && /^[\w./:-]+$/.test(id)) &&
        provider.models.includes(provider.model)
    )
  ) {
    throw new Error('DeepSeek 配置不完整，未修改任何配置')
  }
  const paths = [
    path.join(home, 'settings.yaml'),
    path.join(home, '.credentials.yaml'),
  ]
  const existed = paths.map((file) => fs.existsSync(file))
  const originals = paths.map((file, i) =>
    existed[i] ? fs.readFileSync(file, 'utf8') : ''
  )
  const documents = originals.map((text) => {
    const doc = yaml.parseDocument(text, { uniqueKeys: true })
    if (doc.errors.length || (doc.contents && !yaml.isMap(doc.contents))) {
      throw new Error('现有配置不是有效的 YAML 映射，未修改任何配置')
    }
    return doc
  })
  const [settings, credentials] = documents
  const current = credentials.toJS() ?? {}
  if (Object.keys(current).length && current.version !== 1) {
    throw new Error(
      '现有凭据格式不兼容，未修改任何配置；请先更新 DeepSeek Harness'
    )
  }
  // These are separate credential planes: welcome checks POSIX-named refs,
  // while older pi-ai routes may still use scope/id records. Keep both in sync.
  for (const section of ['refs', 'records']) {
    if (credentials.has(section) && !yaml.isMap(credentials.get(section, true))) {
      throw new Error('现有凭据结构不兼容，未修改任何配置')
    }
  }
  for (const [ref, value] of Object.entries(current.refs ?? {})) {
    if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(ref) || typeof value !== 'string' || !value.length) {
      throw new Error('现有凭据引用格式不兼容，未修改任何配置')
    }
  }
  for (const parts of [['llm-pi-ai'], ['llm-pi-ai', 'providers']]) {
    if (settings.hasIn(parts) && !yaml.isMap(settings.getIn(parts, true))) {
      throw new Error('现有模型配置结构不兼容，未修改任何配置')
    }
  }
  for (const provider of providers) {
    const credentialRef = CREDENTIAL_REF
    settings.setIn(['llm-pi-ai', 'providers', provider.provider_id], {
      displayName: provider.provider_name,
      apiKeyEnv: credentialRef,
      api: 'openai-completions',
      baseURL: provider.base_url,
      compat: { thinkingFormat: 'deepseek' },
      models: provider.models.map((id) => ({ id, name: id })),
    })
    credentials.setIn(['refs', credentialRef], provider.api_key)
    credentials.setIn(['records', `llm-pi-ai/${provider.provider_id}`], {
      kind: 'api-key',
      key: provider.api_key,
    })
  }
  const defaultProvider = providers.find((provider) => provider.provider_id === defaultProviderId)
  settings.set('agent-default-model', { provider: defaultProvider.provider_id, model: defaultProvider.model })
  credentials.set('version', 1)

  fs.mkdirSync(home, { recursive: true })
  const stamp = new Date().toISOString().replaceAll(/[:.]/g, '-')
  for (let i = 0; i < paths.length; i++) {
    if (fs.existsSync(paths[i])) {
      fs.writeFileSync(`${paths[i]}.bak-${stamp}`, originals[i], {
        flag: 'wx',
        mode: 0o600,
      })
    }
  }
  // Prepare both files before replacing either. Credentials are published first,
  // then settings makes the route visible; restore both on an interrupted write.
  const temps = paths.map((file) => `${file}.tokenplus-${process.pid}.tmp`)
  try {
    documents.forEach((doc, i) =>
      fs.writeFileSync(temps[i], doc.toString(), { flag: 'wx', mode: 0o600 })
    )
    fs.renameSync(temps[1], paths[1])
    fs.chmodSync(paths[1], 0o600)
    fs.renameSync(temps[0], paths[0])
  } catch (error) {
    for (let i = 0; i < paths.length; i++) {
      if (existed[i]) {
        fs.writeFileSync(paths[i], originals[i], { mode: 0o600 })
      } else if (fs.existsSync(paths[i])) fs.unlinkSync(paths[i])
    }
    throw error
  } finally {
    for (const file of temps) if (fs.existsSync(file)) fs.unlinkSync(file)
  }
}

async function loadDesktopApp(appRoot) {
  const appRequire = createRequire(path.join(appRoot, 'package.json'))
  const app = appRequire('./package.json')
  if (
    !['deepseek-harness-desktop', '@deepseek-ai/dsh-desktop'].includes(app.name)
  ) {
    throw new Error('请选择 DeepSeek Harness 的安装目录')
  }
  // Official releases keep Harness dependencies under dsh inside app.asar.
  // Electron's Node mode reads that archive without unpacking or modifying it.
  const runtimeRequire =
    app.name === '@deepseek-ai/dsh-desktop'
      ? createRequire(path.join(appRoot, 'dsh', 'package.json'))
      : appRequire
  const yaml = runtimeRequire('yaml')
  const { withFileLock } = await import(
    pathToFileURL(runtimeRequire.resolve('@deepseek-ai/dsh-atomic-write')).href
  )
  const home = path.resolve(
    process.env.DSH_HOME?.trim() || path.join(os.homedir(), '.dsh')
  )
  return { yaml, withFileLock, home }
}

async function main() {
  const [appRoot] = process.argv.slice(2)
  const apiKey = process.env.TOKENPLUS_API_KEY ?? ''
  const model = process.env.TOKENPLUS_MODEL ?? ''
  const baseUrl = process.env.TOKENPLUS_BASE_URL || 'https://api.tokenplus.cn/v1'
  const models = (process.env.TOKENPLUS_MODELS ?? '')
    .split('\n')
    .map((id) => id.trim())
    .filter(Boolean)
  if (!appRoot || !apiKey || !model) {
    throw new Error('缺少配置参数，请通过 deepseek-setup.sh 运行')
  }
  const { yaml, withFileLock, home } = await loadDesktopApp(appRoot)
  const config = {
    default_provider_id: PROVIDER_ID,
    providers: [
      {
        provider_id: PROVIDER_ID,
        provider_name: 'TokenPlus',
        api_key: apiKey,
        base_url: baseUrl,
        model,
        models: models.length ? models : [model],
      },
    ],
  }
  fs.mkdirSync(home, { recursive: true })
  await withFileLock(path.join(home, 'settings.yaml'), () =>
    withFileLock(path.join(home, '.credentials.yaml'), () =>
      applyDeepSeekConfiguration(config, home, yaml)
    )
  )
  process.stdout.write(`  已写入 ${config.providers[0].models.length} 个模型\n`)
}

if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(fs.realpathSync(process.argv[1])).href
) {
  main().catch((error) => {
    process.stderr.write(`${error.message}\n`)
    process.exitCode = 1
  })
}
