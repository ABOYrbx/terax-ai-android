import { SettingsApp } from "@/settings/SettingsApp";
import type { SettingsTab } from "@/modules/settings/openSettingsWindow";

type Props = {
  open: boolean;
  tab: SettingsTab | null;
  onClose: () => void;
};

export function SettingsDialog({ open, tab, onClose }: Props) {
  if (!open) return null;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center">
      <div
        className="absolute inset-0 bg-black/50"
        onClick={onClose}
      />
      <div className="relative z-10 size-full">
        <SettingsApp onClose={onClose} initialTab={tab ?? undefined} />
      </div>
    </div>
  );
}
