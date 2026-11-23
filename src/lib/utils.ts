import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

/** 合并 Tailwind class，冲突项后者覆盖前者（shadcn-vue 约定） */
export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}
