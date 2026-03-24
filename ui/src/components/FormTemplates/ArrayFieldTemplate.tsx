import { DeleteOutlined, PlusOutlined } from '@ant-design/icons';
import type { ArrayFieldTemplateProps } from '@rjsf/utils';
import { Button, Card, Space, Typography } from 'antd';

const { Text } = Typography;

/**
 * Custom ArrayFieldTemplate that uses Ant Design Cards for array items
 * with add/remove buttons.
 */
export function ArrayFieldTemplate(props: ArrayFieldTemplateProps) {
  const {
    title,
    items,
    canAdd,
    onAddClick,
    disabled,
    readonly,
  } = props;

  return (
    <div className="array-field-template" style={{ marginBottom: 16 }}>
      {title && (
        <Text strong style={{ display: 'block', marginBottom: 12 }}>
          {title}
        </Text>
      )}
      
      <Space direction="vertical" style={{ width: '100%' }} size="middle">
        {items.map((element) => (
          <Card
            key={element.key}
            size="small"
            extra={
              element.hasRemove && (
                <Button
                  type="text"
                  danger
                  size="small"
                  icon={<DeleteOutlined />}
                  onClick={element.onDropIndexClick(element.index)}
                  disabled={disabled || readonly || element.disabled}
                >
                  Remove
                </Button>
              )
            }
            style={{ width: '100%' }}
          >
            {element.children}
          </Card>
        ))}
        
        {canAdd && (
          <Button
            type="dashed"
            onClick={onAddClick}
            icon={<PlusOutlined />}
            style={{ width: '100%' }}
            disabled={disabled || readonly}
          >
            Add {title ? title.replace(/s$/, '') : 'Item'}
          </Button>
        )}
      </Space>
    </div>
  );
}
