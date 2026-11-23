<script lang="ts" setup>
import { cn } from "@inspira-ui/plugins";
import { ref } from "vue";

interface Props {
  text?: string;
  class?: string;
}
const props = withDefaults(defineProps<Props>(), {
  text: "Button",
});

const buttonRef = ref<HTMLButtonElement>();
void buttonRef;
</script>

<template>
  <button
    ref="buttonRef"
    :class="
      cn(
        'group relative flex w-full cursor-pointer items-center justify-between gap-2 overflow-hidden rounded-full border bg-transparent px-3 py-1 text-left font-semibold',
        props.class,
      )
    "
  >
    <!-- 底色填充层：从中心放大，四角始终对齐按钮边界 -->
    <span
      class="pointer-events-none absolute inset-0 origin-center scale-0 rounded-[inherit] bg-[var(--color-primary,#0095ff)] opacity-0 transition-all duration-[320ms] ease-out group-hover:scale-100 group-hover:opacity-100"
    />
    <span
      class="relative z-10 whitespace-nowrap transition-all duration-[320ms] ease-out group-hover:translate-x-[2px] group-hover:text-white"
    >
      {{ text }}
    </span>
    <svg
      xmlns="http://www.w3.org/2000/svg"
      width="14"
      height="14"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="2.4"
      stroke-linecap="round"
      stroke-linejoin="round"
      class="relative z-10 shrink-0 translate-x-0 opacity-60 transition-all duration-[320ms] ease-out group-hover:translate-x-[2px] group-hover:opacity-100 group-hover:text-white"
    >
      <path d="M5 12h14" />
      <path d="m12 5 7 7-7 7" />
    </svg>
  </button>
</template>

<style scoped>
button {
  font: inherit;
  line-height: inherit;
  color: inherit;
}
</style>
