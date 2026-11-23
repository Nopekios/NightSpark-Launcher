import { ref } from "vue";

/**
 * 3D 卡片共享鼠标状态（inspira-ui card-3d 依赖）。
 * 派生自官方 composable，暴露 hover 开关供 CardItem 做出场动画。
 */
export function useMouseState() {
  const isMouseEntered = ref(false);

  return {
    x: 0,
    y: 0,
    mouseX: 0,
    mouseY: 0,
    isMouseEntered,
    setMouseEntered(value: boolean) {
      isMouseEntered.value = value;
    },
  };
}
