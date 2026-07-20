import { defineConfig } from 'vitepress'

const base = process.env.VITEPRESS_BASE ?? '/'

export default defineConfig({
  title: 'Ayni',
  description: 'Open-source code quality signals for repositories that use AI agents.',
  base,
  srcExclude: ['initiatives/**'],
  cleanUrls: true,
  themeConfig: {
    socialLinks: [
      { icon: 'github', link: 'https://github.com/gdurandvadas/ayni' },
    ],
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
            {
              text: 'Signals',
              link: '/product/signals',
              items: [
                { text: 'Schema v2', link: '/product/signals/v2' },
                { text: 'Schema v1 (historical)', link: '/product/signals/v1' },
              ],
            },
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
          { text: 'Kotlin', link: '/adapters/kotlin' },
        ],
      },
      { text: 'Contributing', link: '/contributing/adapters' },
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
            {
              text: 'Signals',
              link: '/product/signals',
              items: [
                { text: 'Schema v2', link: '/product/signals/v2' },
                { text: 'Schema v1 (historical)', link: '/product/signals/v1' },
              ],
            },
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
            { text: 'Kotlin', link: '/adapters/kotlin' },
          ],
        },
        {
          text: 'Contributing',
          items: [{ text: 'Language adapters', link: '/contributing/adapters' }],
        },
      ],
    },
  },
})
