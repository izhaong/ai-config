import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

/** 合并 className（shadcn/ui 惯例；Tailwind 接入后用于组件变体） */
export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}
