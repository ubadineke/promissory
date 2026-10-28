import { useMemo } from "react";
import QRCode from "qrcode";

/**
 * A QR code as inline SVG. Always dark on white with a quiet zone, whatever the theme,
 * so every scanner reads it.
 */
export function QrCode({ value, label, size = 224 }: { value: string; label: string; size?: number }) {
  const { path, dim } = useMemo(() => {
    const qr = QRCode.create(value, { errorCorrectionLevel: "M" });
    const n = qr.modules.size;
    const quiet = 4;
    let d = "";
    for (let y = 0; y < n; y++) {
      for (let x = 0; x < n; x++) {
        if (qr.modules.get(y, x)) d += `M${x + quiet} ${y + quiet}h1v1h-1z`;
      }
    }
    return { path: d, dim: n + quiet * 2 };
  }, [value]);

  return (
    <svg
      role="img"
      aria-label={label}
      viewBox={`0 0 ${dim} ${dim}`}
      width={size}
      height={size}
      shapeRendering="crispEdges"
      className="block h-auto max-w-full rounded-xl"
    >
      {/* Fixed colours: a QR code must stay dark-on-light in dark mode too. */}
      <rect width={dim} height={dim} fill="#ffffff" />
      <path d={path} fill="#0b0f14" />
    </svg>
  );
}
