/**
 * The one place that knows how to reach the backend.
 *
 * Every call goes through here so the auth header and the error mapping exist
 * once rather than per caller.
 */

export async function request(path: string, init?: RequestInit): Promise<unknown> {
  const response = await fetch(`/api/v1${path}`, withAuth(init));
  if (!response.ok) {
    throw await toError(response);
  }
  return response.json();
}

/** Attaches the stored token, if there is one. */
function withAuth(init?: RequestInit): RequestInit {
  const token = localStorage.getItem("token");
  return token ? { ...init, headers: { Authorization: `Bearer ${token}` } } : { ...init };
}

/** Turns a failed response into the error the UI shows. */
async function toError(response: Response): Promise<Error> {
  const body = await response.text();
  return new Error(`${response.status}: ${body}`);
}
