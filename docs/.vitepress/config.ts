import { defineConfig } from 'vitepress'

const base = process.env.VITEPRESS_BASE ?? '/ayni/'

export default defineConfig({
  title: 'Ayni',
  description: 'Open-source code quality signals for repositories that use AI agents.',
  base,
  srcExclude: ['initiatives/**'],
  cleanUrls: true,
  themeConfig: {
    search: {
      provider: 'local',
    },
    nav: [
      { text: 'Home', link: '/' },
      { text: 'CLI', link: '/cli' },
      {
        text: 'Product Reference',
        items: [
          { text: 'Configuration', link: '/product/config' },
          { text: 'Signals', link: '/product/signals' },
          { text: 'Runtime', link: '/product/runtime' },
        ],
      },
      {
        text: 'Adapters',
        items: [
          { text: 'Rust', link: '/adapters/rust' },
          { text: 'Node', link: '/adapters/node' },
          { text: 'Go', link: '/adapters/go' },
          { text: 'Python', link: '/adapters/python' },
          { text: 'Template', link: '/adapters/template' },
          { text: 'Kotlin', link: '/adapters/kotlin' },
        ],
      },
    ],
    sidebar: {
      '/': [
        {
          text: 'Docs',
          items: [
            { text: 'Home', link: '/' },
            { text: 'CLI', link: '/cli' },
          ],
        },
        {
          text: 'Product Reference',
          items: [
            { text: 'Configuration', link: '/product/config' },
            { text: 'Signals', link: '/product/signals' },
            { text: 'Runtime', link: '/product/runtime' },
          ],
        },
        {
          text: 'Adapters',
          items: [
            { text: 'Rust', link: '/adapters/rust' },
            { text: 'Node', link: '/adapters/node' },
            { text: 'Go', link: '/adapters/go' },
            { text: 'Python', link: '/adapters/python' },
            { text: 'Template', link: '/adapters/template' },
            { text: 'Kotlin', link: '/adapters/kotlin' },
          ],
        },
      ],
    },
  },
})
