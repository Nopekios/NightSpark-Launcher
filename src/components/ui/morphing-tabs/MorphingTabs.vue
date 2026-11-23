<script lang="ts" setup>
import { cn } from "@inspira-ui/plugins";
interface Props {
  tabs: string[];
  activeTab: string;
  margin?: number;
  class?: string;
  blurStdDeviation?: number;
}

const props = withDefaults(defineProps<Props>(), {
  margin: 20,
  blurStdDeviation: 6,
});
const emit = defineEmits<{
  (e: "update:activeTab", tab: string): void;
}>();
</script>

<template>
  <div
    v-if="props.tabs.length"
    style="filter: url(&quot;#exclusionTabsGoo&quot;)"
    :class="cn(`relative`, props.class)"
  >
    <button
      v-for="tab in props.tabs"
      :key="tab"
      class="bg-primary text-background px-4 py-2 transition-all duration-500"
      :style="{
        margin: `0 ${activeTab === tab ? props.margin : 0}px`,
      }"
      @click="emit('update:activeTab', tab)"
    >
      {{ tab }}
    </button>

    <!--
      这里只是给 filter: url(#exclusionTabsGoo) 提供 defs，本身不参与渲染。
      必须零尺寸 + pointer-events:none：否则这个绝对定位容器里的 svg 会以默认 300x150
      盖在标签下方的页面上，把下方（如资源市场「模组/整合包/光影」分类按钮）的点击吞掉。
    -->
    <div
      class="pointer-events-none absolute h-0 w-0 overflow-hidden"
      aria-hidden="true"
    >
      <svg
        xmlns="http://www.w3.org/2000/svg"
        version="1.1"
        width="0"
        height="0"
      >
        <defs>
          <filter
            id="exclusionTabsGoo"
            x="-50%"
            y="-50%"
            width="200%"
            height="200%"
            color-interpolation-filters="sRGB"
          >
            <feGaussianBlur
              in="SourceGraphic"
              :stdDeviation="blurStdDeviation"
              result="blur"
            />
            <feColorMatrix
              in="blur"
              type="matrix"
              values="
              1 0 0 0 0  
              0 1 0 0 0  
              0 0 1 0 0  
              0 0 0 36 -12"
              result="goo"
            />
            <feComposite
              in="SourceGraphic"
              in2="goo"
              operator="atop"
            />
          </filter>
        </defs>
      </svg>
    </div>
  </div>
</template>

<style></style>
