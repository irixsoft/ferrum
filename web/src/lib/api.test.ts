import { describe, expect, test } from "bun:test";
import { ApiError, parse } from "./api";

describe("parse", () => {
  test("an empty body is undefined whatever the success status", () => {
    expect(parse(202, "Accepted", "")).toBeUndefined();
    expect(parse(204, "No Content", "")).toBeUndefined();
  });

  test("a JSON body is returned as is", () => {
    expect(parse<{ enabled: boolean }>(200, "OK", '{"enabled":true}')).toEqual({ enabled: true });
  });

  test("a refusal carries the server's sentence, or the status when there is none", () => {
    expect(() => parse(409, "Conflict", '{"error":"The firewall is already enabled."}')).toThrow(
      new ApiError(409, "The firewall is already enabled."),
    );
    expect(() => parse(413, "Request Entity Too Large", "<html>nginx</html>")).toThrow(
      new ApiError(413, "413 Request Entity Too Large"),
    );
    expect(() => parse(500, "Internal Server Error", "")).toThrow(
      new ApiError(500, "500 Internal Server Error"),
    );
  });
});
