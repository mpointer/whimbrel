// The part of qrcode@1.5.4 (MIT) the recovery setup uses: the module matrix, drawn by qr.ts.
declare module "qrcode" {
  interface BitMatrix {
    size: number;
    get(row: number, col: number): number | boolean;
  }
  interface QRCodeData {
    modules: BitMatrix;
    version: number;
  }
  const QRCode: {
    create(text: string, options?: { errorCorrectionLevel?: "L" | "M" | "Q" | "H" }): QRCodeData;
  };
  export default QRCode;
}
