'use client';

import { useRef, useState } from 'react';

type RegisterResponse = {
  user: {
    id: string;
    email: string;
    display_name: string | null;
    org_id: string;
    role: string;
  };
  access_token: string;
  refresh_token: string;
  token_type: string;
  expires_in: number;
};

type ApiError = { error: string; message?: string };

export default function SignupForm() {
  const [email, setEmail] = useState('');
  const [password, setPassword] = useState('');
  const [orgName, setOrgName] = useState('');
  const [honeypot, setHoneypot] = useState('');
  const [submitting, setSubmitting] = useState(false);
  const [result, setResult] = useState<RegisterResponse | null>(null);
  const [error, setError] = useState<string | null>(null);
  // Captured at first render — the API route requires the form to be
  // submitted >= 2 s after this. Throws off naive auto-fillers.
  const renderedAtRef = useRef<number>(Date.now());

  async function onSubmit(e: React.FormEvent) {
    e.preventDefault();
    setError(null);
    setSubmitting(true);
    try {
      const res = await fetch('/api/signup', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({
          email,
          password,
          org_name: orgName,
          honeypot,
          rendered_at: renderedAtRef.current,
        }),
      });
      const body = (await res.json()) as RegisterResponse | ApiError;
      if (!res.ok) {
        const msg =
          'message' in body && body.message
            ? body.message
            : 'error' in body
              ? body.error
              : `Server returned ${res.status}`;
        setError(msg);
        return;
      }
      setResult(body as RegisterResponse);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setSubmitting(false);
    }
  }

  if (result) {
    return <SuccessPanel result={result} />;
  }

  return (
    <form onSubmit={onSubmit} className="space-y-5">
      <Field
        id="email"
        label="Email"
        type="email"
        value={email}
        onChange={setEmail}
        placeholder="you@yourcompany.com"
        autoComplete="email"
        required
      />
      <Field
        id="org_name"
        label="Organization name"
        value={orgName}
        onChange={setOrgName}
        placeholder="Acme Inc."
        autoComplete="organization"
        required
      />
      <Field
        id="password"
        label="Password"
        type="password"
        value={password}
        onChange={setPassword}
        placeholder="At least 8 characters"
        autoComplete="new-password"
        required
        minLength={8}
        hint="Used for /auth/login. Choose a long random string and store it in your secret manager — there's no password reset flow during beta."
      />

      {/* Honeypot — real users never see or fill this. Bots usually do. */}
      <div
        aria-hidden="true"
        style={{
          position: 'absolute',
          left: '-10000px',
          width: '1px',
          height: '1px',
          overflow: 'hidden',
        }}
      >
        <label htmlFor="company_website">Company website</label>
        <input
          id="company_website"
          name="company_website"
          type="text"
          tabIndex={-1}
          autoComplete="off"
          value={honeypot}
          onChange={(e) => setHoneypot(e.target.value)}
        />
      </div>

      {error && (
        <div className="rounded-md border border-red-500/40 bg-red-500/10 px-4 py-3 text-sm text-red-200">
          {error}
        </div>
      )}

      <button
        type="submit"
        disabled={submitting}
        className="w-full rounded-md bg-fd-primary px-5 py-3 font-medium text-fd-primary-foreground hover:opacity-90 disabled:opacity-50 transition-opacity"
      >
        {submitting ? 'Creating account…' : 'Create account & get API token'}
      </button>

      <p className="text-xs text-fd-muted-foreground">
        By creating an account you agree to use the hosted API in good faith
        during the beta. There&apos;s no rate limit yet — please don&apos;t
        force us to add one. We&apos;ll give every beta user 30 days notice
        before any pricing kicks in.
      </p>
    </form>
  );
}

function Field({
  id,
  label,
  value,
  onChange,
  type = 'text',
  placeholder,
  autoComplete,
  required,
  minLength,
  hint,
}: {
  id: string;
  label: string;
  value: string;
  onChange: (v: string) => void;
  type?: string;
  placeholder?: string;
  autoComplete?: string;
  required?: boolean;
  minLength?: number;
  hint?: string;
}) {
  return (
    <div>
      <label
        htmlFor={id}
        className="block text-sm font-medium text-fd-foreground"
      >
        {label}
      </label>
      <input
        id={id}
        name={id}
        type={type}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder={placeholder}
        autoComplete={autoComplete}
        required={required}
        minLength={minLength}
        className="mt-1 block w-full rounded-md border border-fd-border bg-fd-background px-3 py-2 text-fd-foreground placeholder-fd-muted-foreground focus:border-fd-primary focus:outline-none focus:ring-1 focus:ring-fd-primary"
      />
      {hint && (
        <p className="mt-1 text-xs text-fd-muted-foreground">{hint}</p>
      )}
    </div>
  );
}

function SuccessPanel({ result }: { result: RegisterResponse }) {
  const [copied, setCopied] = useState<string | null>(null);

  function copy(label: string, value: string) {
    void navigator.clipboard.writeText(value).then(() => {
      setCopied(label);
      setTimeout(() => setCopied((c) => (c === label ? null : c)), 1500);
    });
  }

  return (
    <div className="space-y-6">
      <div className="rounded-md border border-emerald-500/40 bg-emerald-500/10 px-4 py-3 text-sm text-emerald-200">
        Account created. Save these tokens now — the access token expires in 1
        hour; the refresh token expires in 7 days. Use{' '}
        <code className="font-mono">POST /api/v1/auth/login</code> to mint new
        ones.
      </div>

      <SecretRow
        label="User ID"
        value={result.user.id}
        copied={copied === 'user'}
        onCopy={() => copy('user', result.user.id)}
      />
      <SecretRow
        label="Org ID"
        value={result.user.org_id}
        copied={copied === 'org'}
        onCopy={() => copy('org', result.user.org_id)}
      />
      <SecretRow
        label="Access token (1h)"
        value={result.access_token}
        copied={copied === 'access'}
        onCopy={() => copy('access', result.access_token)}
        secret
      />
      <SecretRow
        label="Refresh token (7d)"
        value={result.refresh_token}
        copied={copied === 'refresh'}
        onCopy={() => copy('refresh', result.refresh_token)}
        secret
      />

      <div className="rounded-md border border-fd-border bg-fd-muted/40 p-4">
        <h3 className="text-sm font-semibold">Try it</h3>
        <pre className="mt-2 overflow-x-auto text-xs leading-relaxed font-mono text-fd-muted-foreground">
{`curl https://wraith-browser.vercel.app/api/v1/auth/me \\
  -H "Authorization: Bearer ${result.access_token.slice(0, 24)}…"`}
        </pre>
        <p className="mt-3 text-xs text-fd-muted-foreground">
          See the{' '}
          <a className="underline" href="/docs/enterprise/api">
            full endpoint reference
          </a>{' '}
          for sessions, swarm fan-out, vault, and the rest of the 77
          endpoints.
        </p>
      </div>
    </div>
  );
}

function SecretRow({
  label,
  value,
  copied,
  onCopy,
  secret,
}: {
  label: string;
  value: string;
  copied: boolean;
  onCopy: () => void;
  secret?: boolean;
}) {
  return (
    <div>
      <div className="flex items-center justify-between">
        <span className="text-sm font-medium">{label}</span>
        <button
          type="button"
          onClick={onCopy}
          className="text-xs underline underline-offset-4 hover:text-fd-primary"
        >
          {copied ? 'Copied!' : 'Copy'}
        </button>
      </div>
      <code
        className={`mt-1 block w-full overflow-x-auto rounded-md border border-fd-border bg-fd-muted/40 px-3 py-2 font-mono text-xs ${
          secret ? 'text-fd-muted-foreground' : 'text-fd-foreground'
        }`}
      >
        {value}
      </code>
    </div>
  );
}
