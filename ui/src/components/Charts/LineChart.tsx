import styled from "@emotion/styled";
import { CategoryScale, Chart, Legend, LinearScale, LineController, LineElement, PointElement, Tooltip } from 'chart.js';
import { useEffect, useRef } from 'react';

Chart.register(LineController, CategoryScale, LinearScale, LineElement, PointElement, Tooltip, Legend );

const Container = styled.div`
    display: flex;
    flex: 1;
    min-width: 300px;
    flex-direction: column;
    gap: var(--spacing-lg);
    padding: var(--spacing-lg);
    background-color: var(--color-bg-container);
    border: 1px solid var(--color-border-secondary);
    border-radius: var(--border-radius-md);
`;

const ChartCanvas = styled.div<{ height?: string }>`
    position: relative;
    flex: 1;
    height: ${props => props.height || "250px"};
`;

const ChartTitle = styled.div`
    display: flex;
    align-items: flex-start;
    font-size: 16px;
    font-weight: 600;
    color: var(--color-text-base);
`

interface LineChartDataset { 
    label: string;
    data: number[];
    borderColor: string;
    tension?: number;
    fill?: boolean;
}

interface LineChartProps { 
    title: string;
    labels: string[];
    datasets: LineChartDataset[];
    height?: string;
}

export const LineChart = ({ title, labels, datasets, height }: LineChartProps) => { 
    const canvasRef = useRef<HTMLCanvasElement>(null);
    const chartRef = useRef<Chart | null>(null);

    useEffect(() => { 
        if (!canvasRef.current) return;

        // destroy existing instance to avoid canvas reuse errors
        if (chartRef.current) { 
            chartRef.current.destroy();
        }

        chartRef.current = new Chart(canvasRef.current, { 
            type: 'line',
            data: { 
                labels,
                datasets: datasets.map((ds: LineChartDataset) => ({ 
                    ...ds,
                    fill: ds.fill ?? false,
                    tension: ds.tension ?? 0,
                })),
            },
            options: { 
                responsive: true,
                maintainAspectRatio: false,
                scales: { 
                    y: { beginAtZero: true },
                },
                plugins: { 
                    tooltip: { 
                        enabled: true,
                        mode: 'nearest',
                        intersect: true,
                        callbacks: { 
                            label: (ctx) => { 
                                return `${ctx.dataset.label}: ${ctx.parsed.y}`;
                            }
                        }
                    },
                    legend: { 
                        position: 'top',
                    }
                }
            },
        });

        return () => { 
            chartRef.current?.destroy();
        }
    }, [labels, datasets]);
    
    return (
        <Container>
            <ChartTitle>{title}</ChartTitle>
            <ChartCanvas
                height={height}
            >
                <canvas ref={canvasRef} />
            </ChartCanvas>
        </Container>
    );
}