import { defineConfig } from 'vitepress'
import { withMermaid } from 'vitepress-plugin-mermaid'

const base = process.env.VITEPRESS_BASE ?? '/'

export default withMermaid(defineConfig({
  title: 'Ayni',
  description: 'Reproducible quality signals for repositories and coding agents.',
  base,
  lastUpdated: true,
  cleanUrls: true,
  ignoreDeadLinks: [
    /^https:\/\/example\.com/,
  ],
  themeConfig: {
    nav: [
      { text: 'Getting Started', link: '/getting-started/quickstart' },
      {
        text: 'Guides',
        items: [
          { text: 'How Ayni Works', link: '/getting-started/how-ayni-works' },
          { text: 'Managed Environments', link: '/product/environments' },
          { text: 'Signals', link: '/product/signals' },
          { text: 'Verification', link: '/product/runtime' },
          { text: 'Impact Analysis', link: '/product/impact' },
        ],
      },
      {
        text: 'Reference',
        items: [
          { text: 'Configuration', link: '/product/config' },
          { text: 'CLI', link: '/cli' },
          { text: 'Rust Adapter', link: '/adapters/rust' },
          { text: 'Node Adapter', link: '/adapters/node' },
          { text: 'Go Adapter', link: '/adapters/go' },
          { text: 'Python Adapter', link: '/adapters/python' },
          { text: 'Kotlin Adapter', link: '/adapters/kotlin' },
        ],
      },
    ],
    sidebar: [
      {
        text: 'Getting Started',
        items: [
          { text: 'Introduction', link: '/' },
          { text: 'Installation', link: '/getting-started/installation' },
          { text: 'Quickstart', link: '/getting-started/quickstart' },
          { text: 'How Ayni Works', link: '/getting-started/how-ayni-works' },
        ],
      },
      {
        text: 'Guides',
        items: [
          { text: 'Managed Environments', link: '/product/environments' },
          { text: 'Signals', link: '/product/signals' },
          { text: 'Runtime and Verification', link: '/product/runtime' },
          { text: 'Impact Analysis', link: '/product/impact' },
        ],
      },
      {
        text: 'Reference',
        items: [
          { text: 'Configuration', link: '/product/config' },
          { text: 'CLI', link: '/cli' },
          { text: 'Signal Schema v3', link: '/product/signals/v3' },
          { text: 'Signal Schema v2', link: '/product/signals/v2' },
          { text: 'Signal Schema v1', link: '/product/signals/v1' },
        ],
      },
      {
        text: 'Adapters',
        items: [
          { text: 'Rust', link: '/adapters/rust' },
          { text: 'Node', link: '/adapters/node' },
          { text: 'Go', link: '/adapters/go' },
          { text: 'Python', link: '/adapters/python' },
          { text: 'Kotlin', link: '/adapters/kotlin' },
        ],
      },
      {
        text: 'Contributing',
        items: [
          { text: 'Building Language Adapters', link: '/contributing/adapters' },
        ],
      },
    ],
    socialLinks: [
      { icon: 'github', link: 'https://github.com/gdurandvadas/ayni' },
    ],
    search: {
      provider: 'local',
    },
  },
}))
