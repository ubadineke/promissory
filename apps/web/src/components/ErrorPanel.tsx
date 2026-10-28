import { ApiError, NetworkError } from "@promissory/client";
import { CircleAlert, PlugZap } from "lucide-react";
import { Button, Card } from "./ui";

/** Error state with a way out. Explains how to start `promissory-app` when it is unreachable. */
export function ErrorPanel({ error, onRetry, title }: { error: unknown; onRetry?: () => void; title?: string }) {
  const offline = error instanceof NetworkError || (error instanceof ApiError && (error.status === 502 || error.status === 504));
  return (
    <Card className="px-5 py-6 sm:px-6">
      <div className="flex gap-4">
        <span className="mt-0.5 shrink-0 text-muted">
          {offline ? <PlugZap className="size-5" aria-hidden /> : <CircleAlert className="size-5 text-danger" aria-hidden />}
        </span>
        <div className="min-w-0 space-y-3">
          <div>
            <h2 className="text-base font-bold">{title ?? (offline ? "Can’t reach the Promissory app" : "Something went wrong")}</h2>
            <p className="mt-1 text-sm text-muted text-pretty">
              {offline ? (
                <>
                  This page talks to <code className="font-mono text-fg">promissory-app</code>, which runs on your own device and
                  holds your keys. Start it, then open{" "}
                  <a className="font-medium text-dawn-ink underline underline-offset-2" href="http://127.0.0.1:8738/orders">
                    http://127.0.0.1:8738/orders
                  </a>
                  .
                </>
              ) : error instanceof Error ? (
                error.message
              ) : (
                String(error)
              )}
            </p>
          </div>
          {onRetry && (
            <Button size="sm" onClick={onRetry}>
              Try again
            </Button>
          )}
        </div>
      </div>
    </Card>
  );
}
