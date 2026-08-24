import type { TFunction } from "i18next";

import type { AssetKind, LinkState } from "../types";

export function assetKindLabel(t: TFunction, kind: AssetKind): string {
  return t(`assetKind.${kind}`);
}

export function platformUiLabel(t: TFunction, active: boolean): string {
  return active ? t("platform.active") : t("platform.inactive");
}

export function linkStateLabel(t: TFunction, state: LinkState): string {
  return t(`linkState.${state}`);
}

export function kindDirName(t: TFunction, kind: AssetKind): string {
  return t(`kindDir.${kind}`);
}
