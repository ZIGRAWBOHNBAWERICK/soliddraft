import { useEffect, useRef, useState } from "react";
import type { GalleryItem, LoadTier } from "./types.ts";

function Placeholder() {
  return <span className="qg-placeholder" aria-hidden="true" />;
}

function StillMedia({ item, tier }: { item: GalleryItem; tier: LoadTier }) {
  return (
    <>
      <Placeholder />
      <img
        className="absolute inset-0 size-full object-cover"
        src={item.src}
        alt={item.alt}
        loading={tier === "full" ? "eager" : "lazy"}
        decoding="async"
      />
    </>
  );
}

function HoverVideo({
  item,
  tier,
  playing,
  reducedMotion,
}: {
  item: GalleryItem;
  tier: LoadTier;
  playing: boolean;
  reducedMotion: boolean;
}) {
  const ref = useRef<HTMLVideoElement | null>(null);
  const [videoLoaded, setVideoLoaded] = useState(false);
  const active = playing && !reducedMotion;
  const hasPoster = item.src.length > 0;
  const showVideo = active && videoLoaded;

  useEffect(() => {
    const video = ref.current;
    if (video == null) return;
    if (active) {
      void video.play()?.catch(() => {});
    } else {
      video.pause();
      try {
        video.currentTime = 0;
      } catch {}
    }
  }, [active]);

  return (
    <>
      <Placeholder />
      {hasPoster ? (
        <img
          className="absolute inset-0 size-full object-cover"
          src={item.src}
          alt={item.alt}
          loading={tier === "full" ? "eager" : "lazy"}
          decoding="async"
          style={{ opacity: showVideo ? 0 : 1 }}
        />
      ) : null}
      <video
        ref={ref}
        className="absolute inset-0 size-full object-cover"
        muted
        loop
        playsInline
        preload="none"
        poster={hasPoster ? item.src : undefined}
        onLoadedData={() => setVideoLoaded(true)}
        style={{ opacity: showVideo ? 1 : 0 }}
      >
        <source src={item.videoSrc} type="video/mp4" />
      </video>
    </>
  );
}

export function GalleryMedia({
  item,
  tier,
  playing,
  reducedMotion,
}: {
  item: GalleryItem;
  tier: LoadTier;
  playing: boolean;
  reducedMotion: boolean;
}) {
  return item.kind === "video" ? (
    <HoverVideo item={item} tier={tier} playing={playing} reducedMotion={reducedMotion} />
  ) : (
    <StillMedia item={item} tier={tier} />
  );
}
