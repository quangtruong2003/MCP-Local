import { Menu } from "lucide-react";
import { useState } from "react";
import type { ReactNode } from "react";
import { Button } from "@/components/ui/button";
import { Separator } from "@/components/ui/separator";
import { Sheet, SheetContent, SheetTitle, SheetTrigger } from "@/components/ui/sheet";
import { TooltipProvider } from "@/components/ui/tooltip";
import { VIEW_META, type ViewId } from "../../types";
import { Brand, NavItems, TunnelFooter } from "./sidebar";
import { PageHeader } from "./page-header";
import { PausedBanner } from "./paused-banner";

interface Props {
  page: ViewId;
  onNavigate: (p: ViewId) => void;
  ready: boolean;
  paused: boolean;
  children: ReactNode;
}

export function AppShell({ page, onNavigate, ready, paused, children }: Props) {
  const [mobileNavOpen, setMobileNavOpen] = useState(false);
  const meta = VIEW_META[page];
  const navigate = (p: ViewId) => {
    setMobileNavOpen(false);
    onNavigate(p);
  };

  return (
    <TooltipProvider delayDuration={200}>
      <div className="flex h-screen overflow-hidden">
        {/* Desktop rail: full ≥lg, icons-only md–lg, hidden <md */}
        <aside className="hidden w-60 flex-col border-r bg-card p-3 md:flex max-lg:w-14 max-lg:items-center">
          <Brand />
          <NavItems page={page} onNavigate={navigate} />
          <TunnelFooter ready={ready} />
        </aside>

        <div className="flex min-w-0 flex-1 flex-col">
          {/* Mobile top bar */}
          <div className="flex items-center gap-2 border-b px-3 py-2 md:hidden">
            <Sheet open={mobileNavOpen} onOpenChange={setMobileNavOpen}>
              <SheetTrigger asChild>
                <Button variant="ghost" size="icon" aria-label="Open navigation">
                  <Menu aria-hidden="true" />
                </Button>
              </SheetTrigger>
              <SheetContent side="left" className="w-64 p-3">
                <SheetTitle className="sr-only">Navigation</SheetTitle>
                <Brand />
                <Separator className="my-2" />
                <NavItems page={page} onNavigate={navigate} />
                <TunnelFooter ready={ready} />
              </SheetContent>
            </Sheet>
            <span className="text-sm font-semibold">{meta.title}</span>
          </div>

          <PageHeader title={meta.title} subtitle={meta.subtitle} />
          <PausedBanner paused={paused} />
          <main className="flex-1 overflow-y-auto">
            <div className="mx-auto w-full max-w-5xl px-6 py-6 lg:px-8">{children}</div>
          </main>
        </div>
      </div>
    </TooltipProvider>
  );
}
