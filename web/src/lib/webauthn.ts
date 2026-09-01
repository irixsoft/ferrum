import { ApiError, request } from "./api";

export function supported(): boolean {
  return typeof window !== "undefined" && !!window.PublicKeyCredential;
}

export async function platformAuthenticator(): Promise<boolean> {
  if (!supported()) return false;
  try {
    return await PublicKeyCredential.isUserVerifyingPlatformAuthenticatorAvailable();
  } catch {
    return false;
  }
}

export function toBase64Url(bytes: ArrayBuffer): string {
  let binary = "";
  for (const byte of new Uint8Array(bytes)) binary += String.fromCharCode(byte);
  return btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

export function fromBase64Url(encoded: string): ArrayBuffer {
  const padded = encoded.replace(/-/g, "+").replace(/_/g, "/");
  const binary = atob(padded.padEnd(padded.length + ((4 - (padded.length % 4)) % 4), "="));
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
  return bytes.buffer;
}

export const CANCELLED = "Cancelled.";

type Descriptor = { id: string; type: string; transports?: AuthenticatorTransport[] };

type CreationOptions = Omit<
  PublicKeyCredentialCreationOptions,
  "challenge" | "user" | "excludeCredentials"
> & {
  challenge: string;
  user: Omit<PublicKeyCredentialUserEntity, "id"> & { id: string };
  excludeCredentials?: Descriptor[];
};

type RequestOptions = Omit<PublicKeyCredentialRequestOptions, "challenge" | "allowCredentials"> & {
  challenge: string;
  allowCredentials?: Descriptor[];
};

const descriptors = (list?: Descriptor[]): PublicKeyCredentialDescriptor[] | undefined =>
  list?.map((d) => ({ ...d, id: fromBase64Url(d.id), type: "public-key" as const }));

export async function enroll(enrollmentToken: string): Promise<void> {
  const begun = await begin<{ id: string; publicKey: CreationOptions }>("/auth/register/begin", {
    enrollment: enrollmentToken,
  });

  const credential = (await ceremony(() =>
    navigator.credentials.create({
      publicKey: {
        ...begun.publicKey,
        challenge: fromBase64Url(begun.publicKey.challenge),
        user: { ...begun.publicKey.user, id: fromBase64Url(begun.publicKey.user.id) },
        excludeCredentials: descriptors(begun.publicKey.excludeCredentials),
      },
    }),
  )) as PublicKeyCredential;
  const response = credential.response as AuthenticatorAttestationResponse;

  await finish("/auth/register/finish", {
    id: begun.id,
    enrollment: enrollmentToken,
    credential: {
      id: credential.id,
      rawId: toBase64Url(credential.rawId),
      type: credential.type,
      clientExtensionResults: credential.getClientExtensionResults(),
      response: {
        attestationObject: toBase64Url(response.attestationObject),
        clientDataJSON: toBase64Url(response.clientDataJSON),
        transports: response.getTransports?.() ?? undefined,
      },
    },
  });
}

export async function signIn(): Promise<void> {
  const begun = await begin<{ id: string; publicKey: RequestOptions }>("/auth/login/begin", {});

  const credential = (await ceremony(() =>
    navigator.credentials.get({
      publicKey: {
        ...begun.publicKey,
        challenge: fromBase64Url(begun.publicKey.challenge),
        allowCredentials: descriptors(begun.publicKey.allowCredentials) ?? [],
      },
    }),
  )) as PublicKeyCredential;
  const response = credential.response as AuthenticatorAssertionResponse;

  await finish("/auth/login/finish", {
    id: begun.id,
    credential: {
      id: credential.id,
      rawId: toBase64Url(credential.rawId),
      type: credential.type,
      clientExtensionResults: credential.getClientExtensionResults(),
      response: {
        authenticatorData: toBase64Url(response.authenticatorData),
        clientDataJSON: toBase64Url(response.clientDataJSON),
        signature: toBase64Url(response.signature),
        userHandle: response.userHandle ? toBase64Url(response.userHandle) : null,
      },
    },
  });
}

const begin = <T>(path: string, body: unknown) =>
  request<T>(path, { method: "POST", body: JSON.stringify(body) });

const finish = (path: string, body: unknown) =>
  request<void>(path, { method: "POST", body: JSON.stringify(body) });

async function ceremony(run: () => Promise<Credential | null>): Promise<Credential> {
  let credential: Credential | null;
  try {
    credential = await run();
  } catch (e) {
    if (e instanceof DOMException && (e.name === "NotAllowedError" || e.name === "AbortError")) {
      throw new ApiError(0, CANCELLED);
    }
    if (e instanceof DOMException && e.name === "InvalidStateError") {
      throw new ApiError(0, "That passkey is already registered on this account.");
    }
    throw new ApiError(
      0,
      e instanceof Error ? e.message : "This browser could not complete the passkey step.",
    );
  }
  if (!credential) throw new ApiError(0, CANCELLED);
  return credential;
}
