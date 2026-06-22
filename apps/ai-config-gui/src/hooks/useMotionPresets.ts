import { useReducedMotion } from "motion/react";
import { useMemo } from "react";
import type { Transition, Variants } from "motion/react";

const EASE_OUT: Transition = { duration: 0.2, ease: [0.16, 1, 0.3, 1] };
const SPRING_PANEL: Transition = {
  type: "spring",
  stiffness: 380,
  damping: 32,
} as const;
const INSTANT: Transition = { duration: 0 };

/** 统一动效预设；尊重 prefers-reduced-motion */
export function useMotionPresets() {
  const reduced = useReducedMotion();

  return useMemo(() => {
    const transition = reduced ? INSTANT : EASE_OUT;
    const panelTransition = reduced ? INSTANT : SPRING_PANEL;

    const backdrop = {
      initial: { opacity: reduced ? 1 : 0 },
      animate: { opacity: 1 },
      exit: { opacity: reduced ? 1 : 0 },
      transition,
    };

    const slidePanel = {
      initial: { x: reduced ? 0 : "100%" },
      animate: { x: 0 },
      exit: { x: reduced ? 0 : "100%" },
      transition: panelTransition,
    };

    const scalePanel = {
      initial: { opacity: reduced ? 1 : 0, scale: reduced ? 1 : 0.96, y: reduced ? 0 : 8 },
      animate: { opacity: 1, scale: 1, y: 0 },
      exit: { opacity: reduced ? 1 : 0, scale: reduced ? 1 : 0.98, y: reduced ? 0 : 4 },
      transition,
    };

    const toast = {
      initial: { opacity: reduced ? 1 : 0, y: reduced ? 0 : 16, scale: reduced ? 1 : 0.96 },
      animate: { opacity: 1, y: 0, scale: 1 },
      exit: { opacity: reduced ? 1 : 0, y: reduced ? 0 : 8, scale: reduced ? 1 : 0.98 },
      transition,
    };

    const listContainer: Variants = reduced
      ? { hidden: {}, visible: {} }
      : {
          hidden: {},
          visible: {
            transition: { staggerChildren: 0.025, delayChildren: 0.015 },
          },
        };

    const listItem: Variants = reduced
      ? { hidden: {}, visible: {} }
      : {
          hidden: { opacity: 0, y: 6 },
          visible: {
            opacity: 1,
            y: 0,
            transition: { duration: 0.18, ease: [0.16, 1, 0.3, 1] },
          },
        };

    const fade = {
      initial: { opacity: reduced ? 1 : 0 },
      animate: { opacity: 1 },
      exit: { opacity: reduced ? 1 : 0 },
      transition,
    };

    return {
      backdrop,
      slidePanel,
      scalePanel,
      toast,
      listContainer,
      listItem,
      fade,
      layoutTransition: reduced
        ? INSTANT
        : ({ type: "spring", stiffness: 500, damping: 38 } as const),
    };
  }, [reduced]);
}
