import { defineConfig } from 'vitepress'

export default defineConfig({
  title: "Rustalink",
  description: "High-performance Rust audio server documentation",
  srcDir: './src',
  base: '/Rustalink/',
  cleanUrls: true,
  themeConfig: {
    logo: '/logo.svg',
    nav: [
      { text: 'Docs', link: '/' }
    ],
    sidebar: [
      {
        text: 'Guide',
        items: [
          { text: 'Introduction', link: '/' },
          { text: 'Installation', link: '/guide/installation' },
          { text: 'Docker', link: '/guide/docker' },
          { text: 'Configuration', link: '/guide/configuration' },
          { text: 'Architecture', link: '/guide/architecture' },
          { text: 'Filters', link: '/guide/filters' },
          { text: 'REST API', link: '/guide/api' }
        ]
      }
    ],
    socialLinks: [
      { icon: 'github', link: 'https://github.com/bongodevs/Rustalink' }
    ],
    search: {
      provider: 'local'
    }
  }
})
