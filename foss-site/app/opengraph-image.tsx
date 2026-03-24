import { ImageResponse } from 'next/og';

export const runtime = 'edge';
export const alt = 'Wraith Browser — AI-Agent-First Browser Engine';
export const size = { width: 1200, height: 630 };
export const contentType = 'image/png';

export default function OgImage() {
  return new ImageResponse(
    (
      <div
        style={{
          background: '#09090b',
          width: '100%',
          height: '100%',
          display: 'flex',
          flexDirection: 'column',
          alignItems: 'center',
          justifyContent: 'center',
          fontFamily: 'system-ui, sans-serif',
        }}
      >
        <div
          style={{
            fontSize: 72,
            fontWeight: 700,
            color: '#fafafa',
            marginBottom: 16,
          }}
        >
          Wraith Browser
        </div>
        <div
          style={{
            fontSize: 28,
            color: '#a1a1aa',
            marginBottom: 40,
          }}
        >
          AI-Agent-First Browser Engine
        </div>
        <div
          style={{
            display: 'flex',
            gap: 32,
            fontSize: 20,
            color: '#34d399',
          }}
        >
          <span>130 MCP Tools</span>
          <span>·</span>
          <span>No Chrome</span>
          <span>·</span>
          <span>~50ms / page</span>
          <span>·</span>
          <span>Open Source</span>
        </div>
      </div>
    ),
    { ...size },
  );
}
