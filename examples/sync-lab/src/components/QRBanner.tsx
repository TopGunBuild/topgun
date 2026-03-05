import { useMemo } from 'react';

/**
 * Minimal QR code generator using SVG. Encodes text into a QR code
 * matrix using a simplified version 2 (25x25) format. For demo
 * purposes this produces a recognizable but non-spec-compliant pattern.
 *
 * For a production app you would use a proper QR library, but the spec
 * requires no external npm dependencies outside the Tailwind/Vite/React
 * ecosystem.
 */
function generateQRMatrix(text: string): boolean[][] {
  const size = 25;
  const matrix: boolean[][] = Array.from({ length: size }, () =>
    Array(size).fill(false),
  );

  // Draw finder patterns (top-left, top-right, bottom-left)
  const drawFinder = (row: number, col: number) => {
    for (let r = 0; r < 7; r++) {
      for (let c = 0; c < 7; c++) {
        const isOuter = r === 0 || r === 6 || c === 0 || c === 6;
        const isInner = r >= 2 && r <= 4 && c >= 2 && c <= 4;
        matrix[row + r][col + c] = isOuter || isInner;
      }
    }
  };

  drawFinder(0, 0);
  drawFinder(0, size - 7);
  drawFinder(size - 7, 0);

  // Timing patterns
  for (let i = 8; i < size - 8; i++) {
    matrix[6][i] = i % 2 === 0;
    matrix[i][6] = i % 2 === 0;
  }

  // Encode data bits from the text into remaining cells
  // Simple deterministic fill based on character codes
  let bitIdx = 0;
  const bytes = new TextEncoder().encode(text);
  const bits: boolean[] = [];
  for (const byte of bytes) {
    for (let b = 7; b >= 0; b--) {
      bits.push(((byte >> b) & 1) === 1);
    }
  }

  for (let col = size - 1; col >= 0; col -= 2) {
    if (col === 6) col = 5; // Skip timing column
    for (let row = 0; row < size; row++) {
      for (let c = 0; c < 2; c++) {
        const x = col - c;
        if (x < 0) continue;
        // Skip finder pattern areas and timing
        if (
          (row < 8 && (x < 8 || x >= size - 8)) ||
          (row >= size - 8 && x < 8) ||
          row === 6 ||
          x === 6
        ) {
          continue;
        }
        if (bitIdx < bits.length) {
          matrix[row][x] = bits[bitIdx];
          bitIdx++;
        } else {
          // Padding pattern
          matrix[row][x] = (row + x) % 3 === 0;
        }
      }
    }
  }

  return matrix;
}

function QRCode({ text, size = 100 }: { text: string; size?: number }) {
  const matrix = useMemo(() => generateQRMatrix(text), [text]);
  const cellSize = size / matrix.length;

  return (
    <svg
      width={size}
      height={size}
      viewBox={`0 0 ${size} ${size}`}
      className="rounded"
    >
      <rect width={size} height={size} fill="white" />
      {matrix.map((row, ri) =>
        row.map((cell, ci) =>
          cell ? (
            <rect
              key={`${ri}-${ci}`}
              x={ci * cellSize}
              y={ri * cellSize}
              width={cellSize}
              height={cellSize}
              fill="black"
            />
          ) : null,
        ),
      )}
    </svg>
  );
}

/**
 * Banner at the top of the page encouraging users to open the app
 * in another tab or scan a QR code on their phone.
 */
export function QRBanner() {
  const url = typeof window !== 'undefined' ? window.location.href : '';

  return (
    <div className="flex items-center justify-between rounded-lg bg-surface px-4 py-3">
      <div>
        <p className="text-sm font-medium text-text">
          Open in another tab to see real-time sync
        </p>
        <a
          href={url}
          target="_blank"
          rel="noopener noreferrer"
          className="text-xs text-primary hover:text-primary-dark transition-colors"
        >
          {url}
        </a>
      </div>
      <div className="flex-shrink-0">
        <QRCode text={url} size={64} />
      </div>
    </div>
  );
}
