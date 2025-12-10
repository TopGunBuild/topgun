import satori from 'satori';
import sharp from 'sharp';

interface OgImageOptions {
  title: string;
  description?: string;
  type?: 'default' | 'blog' | 'docs';
}

export async function generateOgImage(options: OgImageOptions): Promise<Buffer> {
  const { title, description, type = 'default' } = options;

  // Load font from Google Fonts CDN
  const interBold = await fetch(
    'https://fonts.gstatic.com/s/inter/v13/UcCO3FwrK3iLTeHuS_fvQtMwCp50KnMw2boKoduKmMEVuFuYMZhrib2Bg-4.ttf'
  ).then((res) => res.arrayBuffer());

  const interRegular = await fetch(
    'https://fonts.gstatic.com/s/inter/v13/UcCO3FwrK3iLTeHuS_fvQtMwCp50KnMw2boKoduKmMEVuLyfMZhrib2Bg-4.ttf'
  ).then((res) => res.arrayBuffer());

  // Color schemes based on type
  const colors = {
    default: { gradient: 'from-blue-600 to-purple-600', accent: '#3b82f6' },
    blog: { gradient: 'from-purple-600 to-pink-600', accent: '#9333ea' },
    docs: { gradient: 'from-emerald-600 to-blue-600', accent: '#10b981' },
  };

  const color = colors[type];

  const svg = await satori(
    {
      type: 'div',
      props: {
        style: {
          height: '100%',
          width: '100%',
          display: 'flex',
          flexDirection: 'column',
          backgroundColor: '#0a0a0a',
          padding: '60px',
          position: 'relative',
        },
        children: [
          // Background gradient
          {
            type: 'div',
            props: {
              style: {
                position: 'absolute',
                top: 0,
                left: 0,
                right: 0,
                bottom: 0,
                background: `linear-gradient(135deg, ${color.accent}15 0%, transparent 50%)`,
              },
            },
          },
          // Grid pattern overlay
          {
            type: 'div',
            props: {
              style: {
                position: 'absolute',
                top: 0,
                left: 0,
                right: 0,
                bottom: 0,
                backgroundImage: 'radial-gradient(circle at 1px 1px, rgba(255,255,255,0.05) 1px, transparent 0)',
                backgroundSize: '40px 40px',
              },
            },
          },
          // Content
          {
            type: 'div',
            props: {
              style: {
                display: 'flex',
                flexDirection: 'column',
                justifyContent: 'space-between',
                height: '100%',
              },
              children: [
                // Top: Logo and type badge
                {
                  type: 'div',
                  props: {
                    style: {
                      display: 'flex',
                      alignItems: 'center',
                      justifyContent: 'space-between',
                    },
                    children: [
                      // Logo
                      {
                        type: 'div',
                        props: {
                          style: {
                            display: 'flex',
                            alignItems: 'center',
                            gap: '12px',
                          },
                          children: [
                            {
                              type: 'div',
                              props: {
                                style: {
                                  width: '48px',
                                  height: '48px',
                                  backgroundColor: 'white',
                                  borderRadius: '8px',
                                  display: 'flex',
                                  alignItems: 'center',
                                  justifyContent: 'center',
                                  fontSize: '24px',
                                  fontWeight: 'bold',
                                  color: '#0a0a0a',
                                },
                                children: 'TG',
                              },
                            },
                            {
                              type: 'span',
                              props: {
                                style: {
                                  fontSize: '28px',
                                  fontWeight: 'bold',
                                  color: 'white',
                                },
                                children: 'TopGun',
                              },
                            },
                          ],
                        },
                      },
                      // Type badge
                      type !== 'default' && {
                        type: 'div',
                        props: {
                          style: {
                            padding: '8px 16px',
                            backgroundColor: `${color.accent}20`,
                            border: `1px solid ${color.accent}40`,
                            borderRadius: '20px',
                            fontSize: '14px',
                            color: color.accent,
                            textTransform: 'uppercase',
                            letterSpacing: '1px',
                          },
                          children: type === 'blog' ? 'Blog' : 'Documentation',
                        },
                      },
                    ].filter(Boolean),
                  },
                },
                // Middle: Title and description
                {
                  type: 'div',
                  props: {
                    style: {
                      display: 'flex',
                      flexDirection: 'column',
                      gap: '20px',
                      flex: 1,
                      justifyContent: 'center',
                    },
                    children: [
                      {
                        type: 'h1',
                        props: {
                          style: {
                            fontSize: title.length > 50 ? '48px' : '56px',
                            fontWeight: 'bold',
                            color: 'white',
                            lineHeight: 1.2,
                            margin: 0,
                          },
                          children: title,
                        },
                      },
                      description && {
                        type: 'p',
                        props: {
                          style: {
                            fontSize: '24px',
                            color: '#a3a3a3',
                            lineHeight: 1.4,
                            margin: 0,
                            maxWidth: '800px',
                          },
                          children: description.length > 120 ? description.slice(0, 120) + '...' : description,
                        },
                      },
                    ].filter(Boolean),
                  },
                },
                // Bottom: URL
                {
                  type: 'div',
                  props: {
                    style: {
                      display: 'flex',
                      alignItems: 'center',
                      justifyContent: 'space-between',
                    },
                    children: [
                      {
                        type: 'span',
                        props: {
                          style: {
                            fontSize: '20px',
                            color: '#737373',
                          },
                          children: 'topgun.dev',
                        },
                      },
                      {
                        type: 'div',
                        props: {
                          style: {
                            width: '120px',
                            height: '4px',
                            background: `linear-gradient(90deg, ${color.accent}, #9333ea)`,
                            borderRadius: '2px',
                          },
                        },
                      },
                    ],
                  },
                },
              ],
            },
          },
        ],
      },
    },
    {
      width: 1200,
      height: 630,
      fonts: [
        {
          name: 'Inter',
          data: interBold,
          weight: 700,
          style: 'normal',
        },
        {
          name: 'Inter',
          data: interRegular,
          weight: 400,
          style: 'normal',
        },
      ],
    }
  );

  // Convert SVG to PNG using sharp
  const png = await sharp(Buffer.from(svg)).png().toBuffer();

  return png;
}
