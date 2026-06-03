import { Button } from "@/components/ui/button";
import { SectionHeader } from "../components/SectionHeader";
import {
  getBootstrapStatus,
  installBootstrap,
  listPackages,
  runApt,
  type BootstrapEvent,
  type BootstrapStatus,
  type InstalledPackage,
} from "@/modules/termux";
import { HugeiconsIcon } from "@hugeicons/react";
import {
  PackageIcon,
  Download04Icon,
  RefreshIcon,
  AlertCircleIcon,
  CheckmarkCircle01Icon,
} from "@hugeicons/core-free-icons";
import { useEffect, useState, useCallback } from "react";

export function PackagesSection() {
  const [status, setStatus] = useState<BootstrapStatus | null>(null);
  const [packages, setPackages] = useState<InstalledPackage[]>([]);
  const [installing, setInstalling] = useState(false);
  const [log, setLog] = useState<string[]>([]);
  const [aptOutput, setAptOutput] = useState("");
  const [aptRunning, setAptRunning] = useState(false);

  const refresh = useCallback(async () => {
    const s = await getBootstrapStatus();
    setStatus(s);
    if (s?.installed) {
      const pkgs = await listPackages();
      setPackages(pkgs);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const handleInstall = async () => {
    setInstalling(true);
    setLog([]);
    try {
      await installBootstrap((event: BootstrapEvent) => {
        if ("log" in event) {
          setLog((prev) => [...prev, event.log.message]);
        } else if ("progress" in event) {
          setLog((prev) => {
            const last = prev.length > 0 ? prev.slice(0, -1) : prev;
            return [
              ...last,
              `${event.progress.message} (${Math.round(event.progress.percent)}%)`,
            ];
          });
        } else if ("error" in event) {
          setLog((prev) => [...prev, `Error: ${event.error.message}`]);
        }
      });
    } catch (e) {
      setLog((prev) => [...prev, `Error: ${e}`]);
    } finally {
      setInstalling(false);
      void refresh();
    }
  };

  const handleApt = async (args: string[]) => {
    setAptRunning(true);
    setAptOutput("");
    try {
      const out = await runApt(args);
      setAptOutput(out);
    } catch (e) {
      setAptOutput(String(e));
    } finally {
      setAptRunning(false);
      void refresh();
    }
  };

  return (
    <div className="flex flex-col gap-6">
      <SectionHeader
        title="Package Manager"
        description="Termux-compatible package management for installing developer tools"
      />

      {status && (
        <div className="flex items-center gap-3 rounded-xl border border-border/60 bg-card/60 p-4">
          {status.installed ? (
            <HugeiconsIcon
              icon={CheckmarkCircle01Icon}
              size={20}
              className="text-green-500"
            />
          ) : (
            <HugeiconsIcon
              icon={AlertCircleIcon}
              size={20}
              className="text-amber-500"
            />
          )}
          <div className="flex min-w-0 flex-col">
            <span className="text-[13px] font-medium">
              {status.installed
                ? "Bootstrap Installed"
                : "Bootstrap Not Installed"}
            </span>
            <span className="text-[11px] text-muted-foreground">
              Architecture: {status.arch}
              {status.prefix ? ` \u00b7 ${status.prefix}` : ""}
            </span>
          </div>
        </div>
      )}

      {!status?.installed && !installing && (
        <div className="flex flex-col gap-2">
          <p className="text-[12px] text-muted-foreground">
            Install the Termux bootstrap to get access to apt, dpkg, and
            hundreds of packages including openssh, git, python, nodejs, and
            more.
          </p>
          <Button onClick={handleInstall} className="gap-1.5 self-start">
            <HugeiconsIcon icon={Download04Icon} size={14} />
            Install Bootstrap ({status?.arch ?? "detecting..."})
          </Button>
        </div>
      )}

      {installing && (
        <div className="flex flex-col gap-2">
          <p className="text-[12px] font-medium text-foreground">
            Installing...
          </p>
          <div className="flex flex-col gap-0.5 rounded-lg border border-border/60 bg-muted/30 p-3 font-mono text-[10.5px] leading-relaxed text-muted-foreground max-h-48 overflow-y-auto">
            {log.map((line, i) => (
              <span key={i}>{line}</span>
            ))}
          </div>
        </div>
      )}

      {status?.installed && (
        <>
          <div className="flex flex-col gap-2">
            <p className="text-[12px] font-medium text-foreground">
              Quick Actions
            </p>
            <div className="flex flex-wrap gap-2">
              <Button
                size="sm"
                variant="outline"
                onClick={() => handleApt(["update"])}
                disabled={aptRunning}
                className="gap-1.5"
              >
                <HugeiconsIcon icon={RefreshIcon} size={12} />
                apt update
              </Button>
              <Button
                size="sm"
                variant="outline"
                onClick={() => handleApt(["list", "--upgradable"])}
                disabled={aptRunning}
                className="gap-1.5"
              >
                List Upgradable
              </Button>
              <Button
                size="sm"
                variant="outline"
                onClick={() => handleApt(["upgrade", "-y"])}
                disabled={aptRunning}
                className="gap-1.5"
              >
                apt upgrade
              </Button>
            </div>
          </div>

          <div className="flex flex-col gap-2">
            <p className="text-[12px] font-medium text-foreground">
              Common Packages
            </p>
            <p className="text-[11px] text-muted-foreground">
              Run these in the terminal to install:
            </p>
            <div className="grid grid-cols-2 gap-1.5">
              {[
                { pkg: "openssh", desc: "SSH client & server" },
                { pkg: "git", desc: "Version control" },
                { pkg: "python", desc: "Python 3" },
                { pkg: "nodejs", desc: "Node.js" },
                { pkg: "build-essential", desc: "C/C++ build tools" },
                { pkg: "vim", desc: "Text editor" },
              ].map(({ pkg, desc }) => (
                <button
                  key={pkg}
                  type="button"
                  onClick={() => handleApt(["install", pkg, "-y"])}
                  disabled={aptRunning}
                  className="flex items-center gap-2 rounded-lg border border-border/60 bg-card/60 px-3 py-2 text-left text-[11px] hover:bg-accent/50 transition-colors disabled:opacity-50"
                >
                  <HugeiconsIcon icon={PackageIcon} size={14} />
                  <div className="flex min-w-0 flex-col">
                    <span className="font-mono font-medium">{pkg}</span>
                    <span className="text-[10px] text-muted-foreground">
                      {desc}
                    </span>
                  </div>
                </button>
              ))}
            </div>
          </div>

          <div className="flex flex-col gap-2">
            <p className="text-[12px] font-medium text-foreground">
              Run apt commands
            </p>
            <p className="text-[11px] text-muted-foreground">
              The easiest way to manage packages is directly in the terminal:
            </p>
            <div className="rounded-lg border border-border/60 bg-muted/30 p-3 font-mono text-[11px] leading-relaxed">
              <span className="text-muted-foreground"># </span>apt update
              <br />
              <span className="text-muted-foreground"># </span>apt install
              openssh git python
              <br />
              <span className="text-muted-foreground"># </span>pkg search
              &lt;query&gt;
              <br />
              <span className="text-muted-foreground"># </span>apt list
              --installed
            </div>
          </div>

          {aptOutput && (
            <div className="flex flex-col gap-1">
              <div className="flex items-center justify-between">
                <span className="text-[12px] font-medium text-foreground">
                  Output
                </span>
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={() => setAptOutput("")}
                >
                  Clear
                </Button>
              </div>
              <pre className="max-h-48 overflow-y-auto rounded-lg border border-border/60 bg-card/60 p-3 font-mono text-[10.5px] leading-relaxed whitespace-pre-wrap">
                {aptOutput}
              </pre>
            </div>
          )}

          {packages.length > 0 && (
            <div className="flex flex-col gap-2">
              <p className="text-[12px] font-medium text-foreground">
                Installed Packages ({packages.length})
              </p>
              <div className="max-h-64 overflow-y-auto rounded-lg border border-border/60">
                {packages.slice(0, 100).map((pkg) => (
                  <div
                    key={pkg.name}
                    className="flex items-center justify-between border-b border-border/40 px-3 py-2 last:border-b-0"
                  >
                    <div className="flex min-w-0 flex-col">
                      <span className="font-mono text-[12px] font-medium">
                        {pkg.name}
                      </span>
                      {pkg.description && (
                        <span className="truncate text-[10px] text-muted-foreground">
                          {pkg.description}
                        </span>
                      )}
                    </div>
                    <span className="ml-2 shrink-0 font-mono text-[10px] text-muted-foreground">
                      {pkg.version}
                    </span>
                  </div>
                ))}
              </div>
            </div>
          )}
        </>
      )}
    </div>
  );
}
