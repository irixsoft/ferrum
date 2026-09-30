import { useMemo } from "react";
import { Link } from "@tanstack/react-router";
import { marked } from "marked";
import { PageTitle } from "@/components/PageTitle";
import { Card, CardBody } from "@/components/ui/Card";
import { useHelp, useHelpTopic } from "@/lib/api";
import { cn } from "@/lib/utils";

export function HelpPage({ topic }: { topic?: string }) {
  const index = useHelp();
  const topics = index.data ?? [];
  const slug = topic ?? topics[0]?.slug;
  const current = useHelpTopic(slug);
  const html = useMemo(
    () => (current.data?.body ? (marked.parse(current.data.body, { async: false }) as string) : ""),
    [current.data?.body],
  );

  return (
    <div>
      <PageTitle above="Conventions a repository follows to run here" title="Help" />
      <div className="grid gap-4 lg:grid-cols-12">
        <nav className="lg:col-span-4 xl:col-span-3 min-w-0">
          <Card>
            <CardBody className="p-2">
              <ul className="grid">
                {topics.map((t) => (
                  <li key={t.slug}>
                    <Link
                      to="/help/$topic"
                      params={{ topic: t.slug }}
                      className={cn(
                        "block px-3 py-2 rounded-control text-[13.5px] transition-colors duration-100",
                        t.slug === slug
                          ? "bg-inset text-ink font-medium"
                          : "text-ink-3 hover:text-ink hover:bg-inset",
                      )}
                    >
                      {t.title}
                    </Link>
                  </li>
                ))}
              </ul>
            </CardBody>
          </Card>
        </nav>
        <article className="lg:col-span-8 xl:col-span-9 min-w-0">
          <Card>
            <CardBody>
              {html ? (
                <div className="help-prose min-w-0" dangerouslySetInnerHTML={{ __html: html }} />
              ) : (
                <p className="text-[13.5px] text-ink-3">
                  {current.isError ? "That topic does not exist." : "Loading."}
                </p>
              )}
            </CardBody>
          </Card>
        </article>
      </div>
    </div>
  );
}
