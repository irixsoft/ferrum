import { expect, test } from "bun:test";
import { fromBase64Url, toBase64Url } from "./webauthn";

test("base64url survives a round trip", () => {
  const bytes = new Uint8Array([0, 1, 250, 251, 252, 253, 254, 255]);
  expect(new Uint8Array(fromBase64Url(toBase64Url(bytes.buffer)))).toEqual(bytes);
});

test("base64url uses no padding and no plus or slash", () => {
  const encoded = toBase64Url(new Uint8Array([251, 255, 190]).buffer);
  expect(encoded).not.toContain("=");
  expect(encoded).not.toContain("+");
  expect(encoded).not.toContain("/");
});

test("standard base64 input still decodes", () => {
  expect(new Uint8Array(fromBase64Url("+/8="))).toEqual(new Uint8Array([251, 255]));
});

test("every unpadded length decodes", () => {
  for (let length = 0; length < 40; length++) {
    const bytes = new Uint8Array(length).map((_, i) => (i * 37) % 256);
    expect(new Uint8Array(fromBase64Url(toBase64Url(bytes.buffer)))).toEqual(bytes);
  }
});

test("a 32-byte challenge encodes to 43 characters", () => {
  expect(toBase64Url(new Uint8Array(32).buffer)).toHaveLength(43);
});
