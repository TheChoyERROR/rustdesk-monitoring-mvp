import { useEffect, useRef } from 'react';
import * as echarts from 'echarts';

import { formatDateTime } from '../lib/time';
import type { SessionActivityTimelineModel } from '../lib/session-activity';

const OUTGOING_COLOR = '#0f766e';
const INCOMING_COLOR = '#f97316';

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

interface SessionActivityChartProps {
  model: SessionActivityTimelineModel;
  onSelectSession?: (sessionId: string) => void;
}

export default function SessionActivityChart({
  model,
  onSelectSession,
}: SessionActivityChartProps) {
  const containerRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!containerRef.current) {
      return undefined;
    }

    const chart = echarts.init(containerRef.current);
    const categoryIndexByUserId = new Map(
      model.users.map((user, index) => [user.userId, index] as const),
    );

    const baseData = model.segments.map((segment) => ({
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

    const renderItem = (params: any, api: any) => {
      const categoryIndex = api.value(0);
      const start = api.coord([api.value(1), categoryIndex]);
      const end = api.coord([api.value(2), categoryIndex]);
      const barHeight = api.size([0, 1])[1] * 0.52;
      const rectShape = echarts.graphic.clipRectByRect(
        {
          x: start[0],
          y: start[1] - barHeight / 2,
          width: Math.max(end[0] - start[0], 2),
          height: barHeight,
        },
        {
          x: params.coordSys.x,
          y: params.coordSys.y,
          width: params.coordSys.width,
          height: params.coordSys.height,
        },
      );

      if (!rectShape) {
        return null;
      }

      return {
        type: 'rect',
        transition: ['shape'],
        shape: rectShape,
        style: api.style(),
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
          color: '#56605f',
        },
      },
      tooltip: {
        trigger: 'item',
        backgroundColor: '#1f2e2d',
        borderWidth: 0,
        textStyle: {
          color: '#fffdf8',
        },
        formatter(params: any) {
          const data = params.data;
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
          color: '#56605f',
        },
        axisLine: {
          lineStyle: {
            color: '#d6d2c8',
          },
        },
        splitLine: {
          lineStyle: {
            color: '#ece8de',
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
          color: '#1f2e2d',
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
            color: OUTGOING_COLOR,
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
            color: INCOMING_COLOR,
            opacity: 0.9,
          },
          data: baseData.filter((segment) => segment.direction === 'incoming'),
        },
      ],
    });

    const handleResize = () => chart.resize();
    window.addEventListener('resize', handleResize);

    const handleClick = (params: any) => {
      const sessionId = params?.data?.sessionId;
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
  }, [model, onSelectSession]);

  return <div ref={containerRef} className="session-timeline-chart" />;
}
