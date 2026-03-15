// @ts-check
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';
import mdx from '@astrojs/mdx';
import mermaid from 'astro-mermaid';

// https://astro.build/config
export default defineConfig({
  base: '/Rustalink/',
  redirects: {
    '/': '/Rustalink/introduction/',
  },
  integrations: [
    starlight({
      title: 'Rustalink',
      favicon: '/favicon.svg',
      description: 'High-performance Rust audio server documentation',
      customCss: ['./src/style/custom.css'],
      logo: {
        src: './src/assets/rastalink.svg',
      },
      social: [
        { icon: 'github', label: 'GitHub', href: 'https://github.com/bongodevs/Rustalink' },
        { icon: 'discord', label: 'Discord', href: 'https://discord.gg/vzjqrUpWxJ' }
      ],
      components: {
        SocialIcons: './src/components/CustomSocialIcons.astro',
      },
      editLink: {
        baseUrl: 'https://github.com/bongodevs/Rustalink/edit/main/docs/',
      },
      sidebar: [
        {
          label: 'Getting Started',
          items: [
            { label: 'Introduction', slug: 'introduction' },
            { label: 'Installation', slug: 'installation' },
            { label: 'Configuration', slug: 'configuration' },
            { label: 'Docker', slug: 'docker' },
            { label: 'Pterodactyl', slug: 'pterodactyl' },
            { label: 'Troubleshooting', slug: 'troubleshooting' },
          ],
        },
        {
          label: 'Core Concepts',
          items: [
            { label: 'Architecture', slug: 'architecture' },
            { label: 'Filters', slug: 'filters' },
            { label: 'Client Libraries', slug: 'clients' },
          ],
        },
        {
          label: 'Reference',
          items: [
            { label: 'REST API', slug: 'api' },
            {
              label: 'WebSocket Events',
              items: [
                { label: 'Track Start', slug: 'events/track-start' },
                { label: 'Track End', slug: 'events/track-end' },
                { label: 'Track Exception', slug: 'events/track-exception' },
                { label: 'Track Stuck', slug: 'events/track-stuck' },
                { label: 'WebSocket Closed', slug: 'events/websocket-closed' },
                {
                  label: 'Lyrics Events',
                  items: [
                    { label: 'Lyrics Found', slug: 'events/lyrics-found' },
                    { label: 'Lyrics Not Found', slug: 'events/lyrics-not-found' },
                    { label: 'Lyrics Line', slug: 'events/lyrics-line' },
                  ],
                },
              ],
            },
          ],
        },
      ],
    }),
    mdx({ optimize: true }),
    mermaid(),
  ],
});
