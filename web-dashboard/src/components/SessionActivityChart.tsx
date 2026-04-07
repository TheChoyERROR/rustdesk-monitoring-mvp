import { useEffect, useRef } from 'react';
import * as echarts from 'echarts';
import type { CallbackDataParams, CustomSeriesRenderItem } from 'echarts/types/dist/shared';

import { formatDateTime } from '../lib/time';
import type { SessionActivityTimelineModel } from '../lib/session-activity';
import { useTheme } from '../useTheme';

function actorTypeLabel(actorType: string): string {
  switch (actorType) {
    case 'agent':
      return 'Agente';
    case 'client':
      return 'Cliente';
    default:
      return 'Sin clasificar';
  }
}

function escapeHtml(value: string): string {
  return value
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;')
    .replaceAll("'", '&#39;');
}

function readThemeColor(styles: CSSStyleDeclaration, variableName: string, fallback: string): string {
  const value = styles.getPropertyValue(variableName).trim();
  return value || fallback;
}

interface SessionActivityChartProps {
  model: SessionActivityTimelineModel;
  onSelectSession?: (sessionId: string) => void;
}

interface SessionActivityDatum {
  value: [number, number, number];
  sessionId: string;
  userId: string;
  displayName: string;
  actorType: string;
  direction: string;
  eventCount: number;
  startLabel: string;
  endLabel: string;
  lastEventType: string;
}

export default function SessionActivityChart({
  model,
  onSelectSession,
}: SessionActivityChartProps) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const { resolvedTheme } = useTheme();

  useEffect(() => {
    if (!containerRef.current) {
      return undefined;
    }

    const chart = echarts.init(containerRef.current);
    const styles = getComputedStyle(document.documentElement);
    const colors = {
      legendText: readThemeColor(styles, '--chart-legend', '#56605f'),
      tooltipBackground: readThemeColor(styles, '--chart-tooltip-bg', '#1f2e2d'),
      tooltipBorder: readThemeColor(styles, '--chart-tooltip-border', 'transparent'),
      tooltipInk: readThemeColor(styles, '--chart-tooltip-ink', '#fffdf8'),
      axisText: readThemeColor(styles, '--chart-axis-label', '#56605f'),
      axisStrong: readThemeColor(styles, '--chart-axis-strong', '#1f2e2d'),
      axisLine: readThemeColor(styles, '--chart-axis-line', '#d6d2c8'),
      gridLine: readThemeColor(styles, '--chart-grid-line', '#ece8de'),
      outgoing: readThemeColor(styles, '--chart-series-outgoing', '#0f766e'),
      incoming: readThemeColor(styles, '--chart-series-incoming', '#f97316'),
    };
    const categoryIndexByUserId = new Map(
      model.users.map((user, index) => [user.userId, index] as const),
    );

    const baseData: SessionActivityDatum[] = model.segments.map((segment) => ({
      value: [
        categoryIndexByUserId.get(segment.userId) ?? 0,
        segment.startMs,
        segment.endMs,
      ],
      sessionId: segment.sessionId,
      userId: segment.userId,
      displayName: segment.displayName,
      actorType: segment.actorType,
      direction: segment.direction,
      eventCount: segment.eventCount,
      startLabel: formatDateTime(new Date(segment.startMs).toISOString()),
      endLabel: formatDateTime(new Date(segment.endMs).toISOString()),
      lastEventType: segment.lastEventType,
    }));

    const renderItem: CustomSeriesRenderItem = (params, api) => {
      const categoryIndex = Number(api.value(0));
      const start = api.coord?.([Number(api.value(1)), categoryIndex]) ?? [0, 0];
      const end = api.coord?.([Number(api.value(2)), categoryIndex]) ?? [0, 0];
      const size = api.size?.([0, 1]);
      const barHeight = (Array.isArray(size) ? size[1] : 0) * 0.52;
      const coordSys = params.coordSys as unknown as {
        x: number;
        y: number;
        width: number;
        height: number;
      };
      const rectShape = echarts.graphic.clipRectByRect(
        {
          x: start[0],
          y: start[1] - barHeight / 2,
          width: Math.max(end[0] - start[0], 2),
          height: barHeight,
        },
        {
          x: coordSys.x,
          y: coordSys.y,
          width: coordSys.width,
          height: coordSys.height,
        },
      );

      if (!rectShape) {
        return null;
      }

      return {
        type: 'rect',
        transition: ['shape'],
        shape: rectShape,
        style: api.style?.(),
      };
    };

    chart.setOption({
      animationDuration: 280,
      backgroundColor: 'transparent',
      grid: {
        left: 180,
        right: 40,
        top: 30,
        bottom: 50,
        containLabel: false,
      },
      legend: {
        top: 0,
        right: 0,
        textStyle: {
          color: colors.legendText,
        },
      },
      tooltip: {
        trigger: 'item',
        backgroundColor: colors.tooltipBackground,
        borderColor: colors.tooltipBorder,
        borderWidth: colors.tooltipBorder === 'transparent' ? 0 : 1,
        textStyle: {
          color: colors.tooltipInk,
        },
        formatter(params: CallbackDataParams) {
          const data = params.data as SessionActivityDatum;
          return [
            `<strong>${escapeHtml(data.displayName)}</strong>`,
            `Usuario: ${escapeHtml(data.userId)}`,
            `Actor: ${escapeHtml(actorTypeLabel(data.actorType))}`,
            `Session: ${escapeHtml(data.sessionId)}`,
            `Direccion: ${escapeHtml(data.direction)}`,
            `Inicio: ${escapeHtml(data.startLabel)}`,
            `Fin: ${escapeHtml(data.endLabel)}`,
            `Eventos: ${escapeHtml(String(data.eventCount))}`,
            `Ultimo evento: ${escapeHtml(data.lastEventType)}`,
          ].join('<br/>');
        },
      },
      xAxis: {
        type: 'time',
        min: new Date(model.rangeStartIso).getTime(),
        max: new Date(model.rangeEndIso).getTime(),
        axisLabel: {
          color: colors.axisText,
        },
        axisLine: {
          lineStyle: {
            color: colors.axisLine,
          },
        },
        splitLine: {
          lineStyle: {
            color: colors.gridLine,
          },
        },
      },
      yAxis: {
        type: 'category',
        inverse: true,
        axisTick: {
          show: false,
        },
        axisLine: {
          show: false,
        },
        axisLabel: {
          color: colors.axisStrong,
          width: 160,
          overflow: 'truncate',
          formatter(value: string) {
            return value;
          },
        },
        data: model.users.map((user) =>
          user.displayName === user.userId ? user.userId : `${user.displayName} (${user.userId})`,
        ),
      },
      series: [
        {
          name: 'Outgoing',
          type: 'custom',
          renderItem,
          encode: {
            x: [1, 2],
            y: 0,
          },
          itemStyle: {
            color: colors.outgoing,
            opacity: 0.9,
          },
          data: baseData.filter((segment) => segment.direction === 'outgoing'),
        },
        {
          name: 'Incoming',
          type: 'custom',
          renderItem,
          encode: {
            x: [1, 2],
            y: 0,
          },
          itemStyle: {
            color: colors.incoming,
            opacity: 0.9,
          },
          data: baseData.filter((segment) => segment.direction === 'incoming'),
        },
      ],
    });

    const handleResize = () => chart.resize();
    window.addEventListener('resize', handleResize);

    const handleClick = (params: CallbackDataParams) => {
      const data = params.data as SessionActivityDatum | undefined;
      const sessionId = data?.sessionId;
      if (sessionId && onSelectSession) {
        onSelectSession(sessionId);
      }
    };

    chart.on('click', handleClick);

    return () => {
      chart.off('click', handleClick);
      window.removeEventListener('resize', handleResize);
      chart.dispose();
    };
  }, [model, onSelectSession, resolvedTheme]);

  return <div ref={containerRef} className="session-timeline-chart" />;
}
